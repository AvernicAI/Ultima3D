//! Blender background-worker client: persistent `blender --background`,
//! JSON-lines RPC where responses are `@JSON@`-prefixed. Mirrors
//! ultima3d/server.py Worker: one in-flight call at a time, hung workers are
//! killed past the deadline so the next call restarts fresh.
//!
//! Pipe I/O blocks, so every call runs on a blocking thread behind an
//! `Arc<std::sync::Mutex<...>>` (the guard crosses the thread boundary,
//! which `tokio::sync::Mutex`'s borrowed guard cannot do).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const PREFIX: &str = "@JSON@";

#[derive(Clone)]
pub struct Worker {
    inner: Arc<Mutex<WorkerInner>>,
    blender: PathBuf,
    worker_py: PathBuf,
    timeout: Duration,
}

struct WorkerInner {
    id: u64,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
}

impl Worker {
    pub fn new(blender: PathBuf, worker_py: PathBuf, timeout_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WorkerInner { id: 0, child: None, stdin: None, stdout: None })),
            blender,
            worker_py,
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    pub async fn call(&self, op: &str, params: Value) -> Result<Value> {
        let inner = Arc::clone(&self.inner);
        let blender = self.blender.clone();
        let worker_py = self.worker_py.clone();
        let op = op.to_string();
        let op_label = op.clone();
        let outcome = tokio::time::timeout(self.timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut guard = inner.lock().map_err(|_| anyhow!("worker lock poisoned"))?;
                guard.ensure(&blender, &worker_py)?;
                guard.roundtrip(&op, params)
            })
            .await
            .map_err(|e| anyhow!("worker task panicked: {e}"))?
        })
        .await;
        match outcome {
            Ok(r) => r,
            Err(_) => {
                self.kill();
                Err(anyhow!(
                    "{op_label} timed out; the hung worker was terminated, so its scene/recipe \
                     state is lost and the next call restarts fresh"
                ))
            }
        }
    }

    /// Kill the worker now (used after a timeout so state can't linger).
    pub fn kill(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.kill();
        }
    }

    pub fn is_running(&self) -> bool {
        self.inner
            .lock()
            .map(|mut g| g.alive())
            .unwrap_or(false)
    }
}

impl WorkerInner {
    fn alive(&mut self) -> bool {
        self.child.as_mut().is_some_and(|c| c.try_wait().unwrap_or(None).is_none())
    }

    fn ensure(&mut self, blender: &std::path::Path, worker_py: &std::path::Path) -> Result<()> {
        if self.alive() {
            return Ok(());
        }
        self.kill();
        if !blender.is_file() {
            anyhow::bail!("blender not found at {}; set ULTIMA3D_BLENDER", blender.display());
        }
        let mut child = Command::new(blender)
            .args(["--background", "--factory-startup", "--python"])
            .arg(worker_py)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("spawning blender worker")?;
        let stdin = child.stdin.take().context("worker stdin")?;
        let stdout = child.stdout.take().map(BufReader::new).context("worker stdout")?;
        self.child = Some(child);
        self.stdin = Some(stdin);
        self.stdout = Some(stdout);
        // First request confirms liveness (mirrors server.py _start).
        self.roundtrip("ping", Value::Object(Default::default()))?;
        Ok(())
    }

    fn kill(&mut self) {
        if let Some(stdin) = self.stdin.take() {
            drop(stdin);
        }
        if let Some(stdout) = self.stdout.take() {
            drop(stdout);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn roundtrip(&mut self, op: &str, params: Value) -> Result<Value> {
        self.id += 1;
        let rid = self.id;
        let line = serde_json::json!({"id": rid, "op": op, "params": params});
        let stdin = self.stdin.as_mut().context("worker not running")?;
        writeln!(stdin, "{}", serde_json::to_string(&line)?)?;
        stdin.flush()?;
        let stdout = self.stdout.as_mut().context("worker not running")?;
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = stdout.read_line(&mut buf)?;
            if n == 0 {
                self.kill();
                anyhow::bail!("blender worker died; check blender stderr");
            }
            let trimmed = buf.trim();
            if !trimmed.starts_with(PREFIX) {
                continue; // Blender progress/log noise on stdout
            }
            let resp: Value = serde_json::from_str(&trimmed[PREFIX.len()..])?;
            if resp.get("id").and_then(Value::as_u64) != Some(rid) {
                continue; // stale response from a crashed previous call
            }
            if resp.get("ok").and_then(Value::as_bool) != Some(true) {
                let err = resp.get("error").and_then(Value::as_str).unwrap_or("unknown");
                let trace = resp.get("trace").and_then(Value::as_str).unwrap_or("");
                anyhow::bail!("{op} failed: {err}\n{trace}");
            }
            return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}
