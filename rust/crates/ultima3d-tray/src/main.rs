//! Ultima3D system-tray controller (Windows).
//!
//! Run the built binary (or `cargo run -p ultima3d-tray`) from anywhere; it
//! owns one `ultima3d-server` MCP subprocess with stdin held open — the same
//! arrangement as the documented background setup:
//!
//!   menu:   Status (text) / Start / Stop / Restart / Open output folder / Quit
//!   icon:   gray = stopped | green = online | amber = busy | blue = idle
//!   chime:  a two-tone beep when the server goes quiet for IDLE_CHIME_SECS
//!            (default 120s) after having been busy — i.e. a job just finished.
//!
//! Status model (no MCP traffic snooping needed):
//!   stopped  server process not running.
//!   busy     the blender worker child is currently burning CPU (a build,
//!            render, bake or validate is actually running).
//!   online   server running, worker quiet, but there was activity recently.
//!   idle     server running, worker quiet for IDLE_CHIME_SECS or more.
//!
//! Busy detection samples the blender.exe child's kernel+user CPU time; a
//! sample above BUSY_CPU_PCT within a poll window counts as working. The idle
//! chime fires once per busy->idle transition (re-armed by the next busy
//! period), never on a server that was simply started and left alone.
//!
//! The tray mark is rendered by Ultima3D itself: `output/icon_work/render_icon.py`
//! builds a beveled block in Blender and renders `assets/mark.png`, which is
//! embedded below and composited over a per-status backdrop at runtime.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub const IDLE_CHIME_SECS: f64 = 120.0;
pub const POLL_SECS: f64 = 5.0;
pub const BUSY_CPU_PCT: f64 = 5.0;
pub const ICON_SIZE: u32 = 32;

fn out_dir() -> PathBuf {
    if let Ok(d) = std::env::var("ULTIMA3D_OUT") {
        return PathBuf::from(d);
    }
    std::env::current_dir()
        .unwrap_or(PathBuf::from("."))
        .join("output")
}

pub fn log(msg: &str) {
    let dir = out_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("tray.log"))
    {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

// ---------------------------------------------------------------------------
// Status model: pure logic, no Windows calls (unit-tested).
// ---------------------------------------------------------------------------

pub const STOPPED: &str = "stopped";
pub const ONLINE: &str = "online";
pub const BUSY: &str = "busy";
pub const IDLE: &str = "idle";

pub struct StatusTracker {
    pub idle_secs: f64,
    pub status: String,
    last_busy: f64,
    chime_armed: bool,
    wants_chime: bool,
}

impl StatusTracker {
    pub fn new(idle_secs: f64) -> Self {
        Self {
            idle_secs,
            status: STOPPED.to_string(),
            last_busy: 0.0,
            chime_armed: false,
            wants_chime: false,
        }
    }

    fn now() -> f64 {
        use std::time::SystemTime as ST;
        ST::now()
            .duration_since(ST::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    pub fn server_down(&mut self) -> &str {
        self.status = STOPPED.to_string();
        self.chime_armed = false;
        self.wants_chime = false;
        &self.status
    }

    pub fn sample(&mut self, running: bool, worker_busy: bool, now: Option<f64>) -> &str {
        let now = now.unwrap_or_else(Self::now);
        if !running {
            return self.server_down();
        }
        if worker_busy {
            self.status = BUSY.to_string();
            self.last_busy = now;
            self.chime_armed = true;
            self.wants_chime = false;
        } else if self.last_busy > 0.0 && now - self.last_busy >= self.idle_secs {
            self.status = IDLE.to_string();
            if self.chime_armed {
                self.wants_chime = true;
                self.chime_armed = false;
            }
        } else {
            self.status = ONLINE.to_string();
        }
        &self.status
    }

    pub fn take_chime(&mut self) -> bool {
        let c = self.wants_chime;
        self.wants_chime = false;
        c
    }
}

// ---------------------------------------------------------------------------
// Icons: the Blender-rendered mark composited over a per-status backdrop.
// ---------------------------------------------------------------------------

pub const ICON_COLORS: &[(&str, (u8, u8, u8))] = &[
    (STOPPED, (128, 128, 128)),
    (ONLINE, (46, 160, 67)),
    (BUSY, (230, 150, 20)),
    (IDLE, (50, 120, 220)),
];

const MARK_PNG: &[u8] = include_bytes!("../assets/mark.png");

fn decode_mark() -> (Vec<u8>, u32, u32) {
    let dec = png::Decoder::new(MARK_PNG);
    let mut reader = dec.read_info().expect("tray mark PNG must decode");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .expect("tray mark PNG must have a frame");
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize].to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for px in buf[..(w * h * 3) as usize].chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for px in buf[..(w * h * 2) as usize].chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for g in buf[..(w * h) as usize].iter() {
                out.extend_from_slice(&[*g, *g, *g, 255]);
            }
            out
        }
        png::ColorType::Indexed => panic!("tray mark PNG must not be indexed"),
    };
    (rgba, w, h)
}

/// Composite the white mark over a status-colored disc. Returns 32x32 RGBA.
pub fn status_rgba(status: &str) -> Vec<u8> {
    let (r, g, b) = ICON_COLORS
        .iter()
        .find(|(s, _)| *s == status)
        .map(|(_, c)| *c)
        .unwrap_or((128, 128, 128));
    let (mark, mw, mh) = decode_mark();
    let size = ICON_SIZE as usize;
    let mut out = vec![0u8; size * size * 4];
    let cx = size as f64 / 2.0;
    let rad = size as f64 / 2.0 - 1.0;
    // Mark occupies the middle ~70% of the icon.
    let mark_box = (size as f64 * 0.70) as usize;
    let mark_off = (size - mark_box) / 2;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 - cx + 0.5;
            let dy = y as f64 - cx + 0.5;
            let d = (dx * dx + dy * dy).sqrt();
            let coverage = ((rad + 0.5 - d).clamp(0.0, 1.0) * 255.0) as f64;
            if coverage <= 0.0 {
                continue;
            }
            // Subtle vertical gradient on the disc.
            let shade = 1.0 + 0.10 * (0.5 - y as f64 / size as f64);
            let mut br = ((r as f64 * shade).clamp(0.0, 255.0)) as u8;
            let mut bg = ((g as f64 * shade).clamp(0.0, 255.0)) as u8;
            let mut bb = ((b as f64 * shade).clamp(0.0, 255.0)) as u8;
            let mut ba = coverage as u8;
            // Paste the mark (nearest-neighbor) where it covers.
            if x >= mark_off && y >= mark_off && x < mark_off + mark_box && y < mark_off + mark_box
            {
                let mx = ((x - mark_off) as u64 * mw as u64 / mark_box as u64) as usize;
                let my = ((y - mark_off) as u64 * mh as u64 / mark_box as u64) as usize;
                let i = (my * mw as usize + mx) * 4;
                let ma = mark[i + 3] as f64 / 255.0;
                if ma > 0.0 {
                    let t = ma * (mark[i + 3] as f64 / 255.0);
                    br = (mark[i] as f64 * t + br as f64 * (1.0 - t)) as u8;
                    bg = (mark[i + 1] as f64 * t + bg as f64 * (1.0 - t)) as u8;
                    bb = (mark[i + 2] as f64 * t + bb as f64 * (1.0 - t)) as u8;
                    ba = ((255.0 * (1.0 - (1.0 - ma) * (1.0 - coverage / 255.0))) as u8).max(ba);
                }
            }
            let o = (y * size + x) * 4;
            out[o..o + 4].copy_from_slice(&[br, bg, bb, ba]);
        }
    }
    out
}

pub fn status_icon(status: &str) -> tray_icon::Icon {
    tray_icon::Icon::from_rgba(status_rgba(status), ICON_SIZE, ICON_SIZE).expect("valid 32x32 RGBA")
}

// ---------------------------------------------------------------------------
// Server process management.
// ---------------------------------------------------------------------------

pub fn resolve_server_exe() -> PathBuf {
    if let Ok(p) = std::env::var("ULTIMA3D_SERVER") {
        return PathBuf::from(p);
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let sibling = dir.join("ultima3d-server.exe");
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    PathBuf::from("ultima3d-server.exe")
}

pub struct ServerManager {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    child: Option<Child>,
}

impl ServerManager {
    pub fn new() -> Self {
        let exe = resolve_server_exe();
        let cwd = std::env::current_dir().unwrap_or(PathBuf::from("."));
        Self {
            command: vec![exe.to_string_lossy().into_owned()],
            cwd,
            child: None,
        }
    }

    pub fn with_command(command: Vec<String>, cwd: PathBuf) -> Self {
        Self {
            command,
            cwd,
            child: None,
        }
    }

    pub fn start(&mut self) -> bool {
        if self.is_running() {
            return true;
        }
        let dir = out_dir();
        let _ = std::fs::create_dir_all(&dir);
        let logf = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("server_run.log"))
            .ok();
        let stdout: Stdio = logf.map(Stdio::from).unwrap_or_else(Stdio::null);
        match Command::new(&self.command[0])
            .args(&self.command[1..])
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                log(&format!("tray: server started pid={}", child.id()));
                self.child = Some(child);
                true
            }
            Err(e) => {
                log(&format!("tray: server start failed: {e}"));
                false
            }
        }
    }

    pub fn stop(&mut self) -> bool {
        let mut child = match self.child.take() {
            Some(c) => c,
            None => return true,
        };
        let _ = child.kill();
        let _ = child.wait();
        log("tray: server stopped");
        true
    }

    pub fn restart(&mut self) -> bool {
        self.stop();
        std::thread::sleep(Duration::from_secs(1));
        self.start()
    }

    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => c.try_wait().map(|s| s.is_none()).unwrap_or(false),
            None => false,
        }
    }

    pub fn pid(&mut self) -> Option<u32> {
        if self.is_running() {
            self.child.as_ref().map(|c| c.id())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Worker-busy sampling: CPU-% of blender.exe children via ToolHelp +
// GetProcessTimes. Unknown/quiet samples read as 0.0, never an error.
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub struct WorkerSampler {
    prev: HashMap<u32, (Instant, f64)>,
}

#[cfg(windows)]
impl WorkerSampler {
    pub fn new() -> Self {
        Self {
            prev: HashMap::new(),
        }
    }

    fn process_cpu_secs(pid: u32) -> Option<f64> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid).ok()?;
            let mut _c = FILETIME::default();
            let mut _e = FILETIME::default();
            let mut k = FILETIME::default();
            let mut u = FILETIME::default();
            GetProcessTimes(h, &mut _c, &mut _e, &mut k, &mut u).ok()?;
            let k64 = ((k.dwHighDateTime as u64) << 32) | k.dwLowDateTime as u64;
            let u64 = ((u.dwHighDateTime as u64) << 32) | u.dwLowDateTime as u64;
            Some((k64 + u64) as f64 / 10_000_000.0)
        }
    }

    fn blender_children(parent_pid: u32) -> Vec<u32> {
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        let mut out = Vec::new();
        unsafe {
            let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(h) => h,
                Err(_) => return out,
            };
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut pe = PROCESSENTRY32W::default();
            pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut pe).is_err() {
                let _ = windows::Win32::Foundation::CloseHandle(snap);
                return out;
            }
            loop {
                let len = pe
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe.szExeFile.len());
                let name = String::from_utf16_lossy(&pe.szExeFile[..len]);
                if name.eq_ignore_ascii_case("blender.exe") && pe.th32ParentProcessID == parent_pid
                {
                    out.push(pe.th32ProcessID);
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
            let _ = windows::Win32::Foundation::CloseHandle(snap);
        }
        out
    }

    pub fn busy_children_cpu(&mut self, parent_pid: Option<u32>) -> f64 {
        let parent = match parent_pid {
            Some(p) => p,
            None => return 0.0,
        };
        let children = Self::blender_children(parent);
        let now = Instant::now();
        let mut best: f64 = 0.0;
        let mut seen = std::collections::HashSet::new();
        for pid in children {
            let t = match Self::process_cpu_secs(pid) {
                Some(t) => t,
                None => continue,
            };
            seen.insert(pid);
            if let Some((prev_now, prev_t)) = self.prev.get(&pid) {
                let dt = now.duration_since(*prev_now).as_secs_f64();
                if dt > 0.0 {
                    best = best.max((t - prev_t) / dt * 100.0);
                }
            }
            self.prev.insert(pid, (now, t));
        }
        self.prev.retain(|pid, _| seen.contains(pid));
        best
    }
}

// ---------------------------------------------------------------------------
// Chime: two-tone beep.
// ---------------------------------------------------------------------------

pub fn chime() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::Beep;
        let _ = Beep(880, 220);
        std::thread::sleep(Duration::from_millis(120));
        let _ = Beep(1175, 320);
    }
    log("tray: idle chime");
}

// ---------------------------------------------------------------------------
// App: menu + icon + poll loop.
// ---------------------------------------------------------------------------

fn single_instance_guard() -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    unsafe {
        match CreateMutexW(None, false, w!("Ultima3D_Tray_Singleton")) {
            Ok(h) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = windows::Win32::Foundation::CloseHandle(h);
                    None
                } else {
                    Some(h)
                }
            }
            Err(_) => None,
        }
    }
}

struct TrayApp {
    manager: ServerManager,
    tracker: StatusTracker,
    #[cfg(windows)]
    sampler: WorkerSampler,
    status: String,
}

impl TrayApp {
    fn tooltip(status: &str) -> String {
        format!("Ultima3D MCP — {status}")
    }
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let _mutex = match single_instance_guard() {
        Some(h) => h,
        None => {
            log("tray: another instance is already running");
            return 1;
        }
    };

    let menu = Menu::new();
    let status_item = MenuItem::new("Status: stopped", false, None);
    let start_item = MenuItem::new("Start server", true, None);
    let stop_item = MenuItem::new("Stop server", false, None);
    let restart_item = MenuItem::new("Restart server", false, None);
    let log_item = MenuItem::new("Open output folder", true, None);
    let quit_item = MenuItem::new("Quit tray", true, None);
    if menu
        .append_items(&[
            &status_item,
            &PredefinedMenuItem::separator(),
            &start_item,
            &stop_item,
            &restart_item,
            &PredefinedMenuItem::separator(),
            &log_item,
            &quit_item,
        ])
        .is_err()
    {
        return 1;
    }

    let mut app = TrayApp {
        manager: ServerManager::new(),
        tracker: StatusTracker::new(IDLE_CHIME_SECS),
        #[cfg(windows)]
        sampler: WorkerSampler::new(),
        status: STOPPED.to_string(),
    };

    let tray: TrayIcon = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(TrayApp::tooltip(STOPPED))
        .with_icon(status_icon(STOPPED))
        .build()
    {
        Ok(t) => t,
        Err(e) => {
            log(&format!("tray: icon build failed: {e:?}"));
            return 1;
        }
    };

    app.manager.start();
    app.status = ONLINE.to_string();
    app.tracker.sample(true, false, None);
    apply_state(
        &tray,
        &status_item,
        &start_item,
        &stop_item,
        &restart_item,
        &app.status,
    );
    log("tray: started");

    let menu_rx = MenuEvent::receiver();
    let tray_rx = TrayIconEvent::receiver();

    let mut last_poll = Instant::now() - Duration::from_secs(99);
    loop {
        // Menu commands.
        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id() == start_item.id() {
                app.manager.start();
            } else if ev.id() == stop_item.id() {
                app.manager.stop();
            } else if ev.id() == restart_item.id() {
                app.manager.restart();
            } else if ev.id() == log_item.id() {
                let _ = Command::new("explorer").arg(out_dir()).spawn();
            } else if ev.id() == quit_item.id() {
                app.manager.stop();
                log("tray: exited");
                return 0;
            }
        }
        // Left-click toggles start/stop (right-click opens the menu natively).
        while let Ok(ev) = tray_rx.try_recv() {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = ev
            {
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    if app.manager.is_running() {
                        app.manager.stop();
                    } else {
                        app.manager.start();
                    }
                }
            }
        }
        // Status poll.
        if last_poll.elapsed().as_secs_f64() >= POLL_SECS {
            last_poll = Instant::now();
            let running = app.manager.is_running();
            #[cfg(windows)]
            let busy = if running {
                app.sampler.busy_children_cpu(app.manager.pid()) >= BUSY_CPU_PCT
            } else {
                false
            };
            #[cfg(not(windows))]
            let busy = false;
            app.status = app.tracker.sample(running, busy, None).to_string();
            if app.tracker.take_chime() {
                chime();
            }
            apply_state(
                &tray,
                &status_item,
                &start_item,
                &stop_item,
                &restart_item,
                &app.status,
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn apply_state(
    tray: &TrayIcon,
    status_item: &MenuItem,
    start_item: &MenuItem,
    stop_item: &MenuItem,
    restart_item: &MenuItem,
    status: &str,
) {
    let running = status != STOPPED;
    let _ = tray.set_icon(Some(status_icon(status)));
    let _ = tray.set_tooltip(Some(TrayApp::tooltip(status)));
    status_item.set_text(format!("Status: {status}"));
    start_item.set_enabled(!running);
    stop_item.set_enabled(running);
    restart_item.set_enabled(running);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    fn isolate_test_output() {
        std::env::set_var("ULTIMA3D_OUT", std::env::temp_dir().join("ultima3d-test"));
    }

    #[test]
    fn status_lifecycle() {
        isolate_test_output();
        let mut t = StatusTracker::new(120.0);
        assert_eq!(t.status, STOPPED);
        t.sample(false, false, Some(1000.0));
        assert_eq!(t.status, STOPPED);
        t.sample(true, false, Some(1000.0));
        assert_eq!(t.status, ONLINE);
        assert!(!t.take_chime(), "no chime without ever being busy");
        t.sample(true, true, Some(1001.0));
        assert_eq!(t.status, BUSY);
        t.sample(true, false, Some(1010.0));
        assert_eq!(t.status, ONLINE);
        assert!(!t.take_chime(), "still no chime before 120s");
        t.sample(true, false, Some(1001.0 + 121.0));
        assert_eq!(t.status, IDLE);
        assert!(t.take_chime(), "idle chime fires");
        assert!(!t.take_chime(), "chime fires only once");
        t.sample(true, true, Some(1200.0));
        assert_eq!(t.status, BUSY);
        t.sample(true, false, Some(1200.0 + 121.0));
        assert!(t.take_chime(), "second idle chimes again");
        t.server_down();
        assert_eq!(t.status, STOPPED);
        assert!(!t.take_chime());
    }

    #[test]
    fn icons_are_distinct_32px_with_transparent_corners() {
        isolate_test_output();
        let mut seen = std::collections::HashSet::new();
        for (name, _) in ICON_COLORS {
            let rgba = status_rgba(name);
            assert_eq!(rgba.len(), 32 * 32 * 4, "icon {name} is 32x32 RGBA");
            // Corner pixel fully transparent (outside the disc).
            assert_eq!(rgba[3], 0, "icon {name} corner transparent");
            // Center pixel opaque (disc + mark).
            let c = (16 * 32 + 16) * 4 + 3;
            assert_eq!(rgba[c], 255, "icon {name} center opaque");
            seen.insert(rgba);
        }
        assert_eq!(seen.len(), 4, "icons differ by status color");
    }

    #[test]
    fn process_manager_holds_a_dummy_child() {
        isolate_test_output();
        // ping -t runs until killed: a stand-in for the stdin-held server.
        let mut mgr = ServerManager::with_command(
            vec![
                "ping".to_string(),
                "-t".to_string(),
                "127.0.0.1".to_string(),
            ],
            std::env::temp_dir(),
        );
        assert!(!mgr.is_running());
        assert_eq!(mgr.pid(), None);
        assert!(mgr.start());
        std::thread::sleep(Duration::from_millis(500));
        assert!(mgr.is_running());
        assert!(mgr.pid().is_some());
        assert!(mgr.start(), "double start is idempotent");
        assert!(mgr.stop());
        assert!(!mgr.is_running());
        assert!(mgr.restart());
        std::thread::sleep(Duration::from_millis(500));
        assert!(mgr.is_running());
        assert!(mgr.stop());
    }

    #[test]
    fn log_never_panics() {
        isolate_test_output();
        log("tray_test ping");
    }

    #[test]
    fn server_exe_resolution_prefers_env() {
        std::env::set_var("ULTIMA3D_SERVER", "C:\\fake\\ultima3d-server.exe");
        assert_eq!(
            resolve_server_exe(),
            PathBuf::from("C:\\fake\\ultima3d-server.exe")
        );
        std::env::remove_var("ULTIMA3D_SERVER");
    }
}
