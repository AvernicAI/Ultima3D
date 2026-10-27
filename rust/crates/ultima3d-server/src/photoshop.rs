//! Photoshop CS3 backend. Mirrors ultima3d/photoshop/bridge.py:
//! portable CS3 registers no COM server, so live control goes through the
//! ExtendScript TCP bridge (one JSON command per connection, reply before
//! close). Texture synthesis lives in textures.rs (headless, no Photoshop).

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const DEFAULT_EXE: &str =
    r"C:\Users\buffb\Desktop\GitHub\Adobe Photoshop CS3 Portable\App\Photoshop.exe";
const DEFAULT_PORT: u16 = 8767;

pub fn exe_path() -> PathBuf {
    PathBuf::from(std::env::var("ULTIMA3D_PHOTOSHOP").unwrap_or_else(|_| DEFAULT_EXE.to_string()))
}

pub fn exe_exists() -> bool {
    exe_path().is_file()
}

pub fn port() -> u16 {
    std::env::var("ULTIMA3D_PHOTOSHOP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn scripts_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or(PathBuf::from("."))
        .join("ultima3d")
        .join("photoshop")
        .join("scripts")
}

pub fn preset_names() -> Vec<String> {
    let mut out: Vec<String> = vec![];
    if let Ok(entries) = std::fs::read_dir(scripts_dir()) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("jsx") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

fn daemon_script() -> PathBuf {
    std::env::current_dir()
        .unwrap_or(PathBuf::from("."))
        .join("ultima3d")
        .join("photoshop")
        .join("daemon")
        .join("ultima3d_bridge.jsx")
}

/// Quick connect test: is the bridge script running inside Photoshop?
pub fn live(timeout: Duration) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port()).parse().unwrap(),
        timeout,
    )
    .is_ok()
}

/// Run ExtendScript in the open Photoshop via the bridge; return its reply.
pub fn call(code: Option<&str>, op: &str, timeout: Duration) -> Result<Value> {
    let p = port();
    let req = serde_json::json!({"id": 1, "op": op, "code": code.unwrap_or("")});
    let mut s = TcpStream::connect_timeout(
        &format!("127.0.0.1:{p}").parse().unwrap(),
        Duration::from_secs(10).min(timeout),
    )
    .map_err(|_| {
        anyhow!(
            "Photoshop bridge is not listening on 127.0.0.1:{p}. In the open \
             Photoshop: File > Scripts > Browse > {}",
            daemon_script().display()
        )
    })?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    s.write_all(req.to_string().as_bytes())?;
    let _ = s.shutdown(Shutdown::Write);
    let mut buf = Vec::new();
    s.read_to_end(&mut buf)?;
    let raw = String::from_utf8_lossy(&buf);
    if raw.trim().is_empty() {
        anyhow::bail!(
            "empty reply from the Photoshop bridge — reload the current daemon: \
             quit the old bridge, then File > Scripts > Browse > {}",
            daemon_script().display()
        );
    }
    let resp: Value =
        serde_json::from_str(&raw).map_err(|_| anyhow!("unreadable reply from the Photoshop bridge: {raw:.120}"))?;
    if resp.get("ok").and_then(Value::as_bool) != Some(true) {
        let err = resp.get("error").and_then(Value::as_str).unwrap_or("unknown");
        anyhow::bail!("ExtendScript error: {err}");
    }
    Ok(resp)
}

const FILLS: [(&str, &str); 3] = [
    ("white", "DocumentFill.WHITE"),
    ("transparent", "DocumentFill.TRANSPARENT"),
    ("background", "DocumentFill.BACKGROUNDCOLOR"),
];

/// ExtendScript that creates a new RGB document and reports it back.
pub fn build_create(width: i64, height: i64, background: &str, name: &str) -> Result<String> {
    let fill = FILLS
        .iter()
        .find(|(k, _)| *k == background.to_lowercase().as_str())
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            anyhow!("unknown background {background:?}; choose from white/transparent/background")
        })?;
    if !(1..=10000).contains(&width) || !(1..=10000).contains(&height) {
        anyhow::bail!("width/height must each be 1..10000");
    }
    let safe: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let safe = if safe.is_empty() { "ultima3d".to_string() } else { safe };
    Ok(format!(
        "var d = app.documents.add({w}, {h}, 72, \"{safe}\", NewDocumentMode.RGB, {fill}); \
         \"created \" + d.name + \" \" + d.width + \"x\" + d.height",
        w = width,
        h = height
    ))
}

/// Fire-and-forget a JSX preset (by stem) or inline code via Photoshop.exe.
pub fn run_script(name_or_code: &str) -> Result<Value> {
    let exe = exe_path();
    if !exe.is_file() {
        anyhow::bail!("Photoshop.exe not found at {}; set ULTIMA3D_PHOTOSHOP", exe.display());
    }
    let candidate = scripts_dir().join(format!("{name_or_code}.jsx"));
    let (code, source) = if candidate.is_file() {
        let code = std::fs::read_to_string(&candidate)?;
        (code, format!("preset:{}", candidate.file_stem().and_then(|s| s.to_str()).unwrap_or("?")))
    } else {
        (name_or_code.to_string(), "inline".to_string())
    };
    if !code.contains("app.") && !code.contains("activeDocument") {
        anyhow::bail!("refusing to launch Photoshop with code that never touches app/activeDocument");
    }
    let staged = std::env::temp_dir().join(format!(
        "ultima3d_{}_{}.jsx",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    std::fs::write(&staged, code).context("staging jsx")?;
    std::process::Command::new(&exe)
        .arg(&staged)
        .spawn()
        .map_err(|e| anyhow!("failed to launch Photoshop.exe: {e}"))?;
    Ok(serde_json::json!({"launched": true, "source": source,
                          "script": staged.to_string_lossy()}))
}

pub fn status() -> Value {
    let exe = exe_path();
    serde_json::json!({
        "ok": true,
        "exe": exe.to_string_lossy(),
        "exe_present": exe.is_file(),
        "com_version": Value::Null,
        "com_note": "portable CS3 registers no COM server; live control uses the bridge script",
        "live": live(Duration::from_secs(1)),
        "port": port(),
        "bridge_script": daemon_script().to_string_lossy(),
        "scripts": preset_names(),
    })
}
