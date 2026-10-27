//! Godot editor bridge client: one JSON POST per tool; screenshot payloads
//! are validated (capture dir + .png + 20 MiB cap) and inlined as base64.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

pub const SCREENSHOT_MAX_BYTES: u64 = 20 * 1024 * 1024;

pub fn bridge_url() -> String {
    std::env::var("GODOT_MCP_URL").unwrap_or_else(|_| "http://127.0.0.1:8765/mcp".to_string())
}

/// Absolute capture dir ({ULTIMA3D_OUT}/godot_captures), created on demand.
pub fn capture_dir() -> PathBuf {
    let base = std::env::var("ULTIMA3D_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or(PathBuf::from("."))
                .join("output")
        });
    base.join("godot_captures")
}

/// Lexically normalize an absolute path (resolves `.`/`..`, no I/O).
fn normalize(p: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        use std::path::Component::*;
        match comp {
            CurDir => {}
            ParentDir => {
                out.pop();
            }
            _ => out.push(comp.as_os_str()),
        }
    }
    out
}

fn absolutize(p: &std::path::Path) -> PathBuf {
    if p.is_absolute() {
        normalize(p.to_path_buf())
    } else {
        let cwd = std::env::current_dir().unwrap_or(PathBuf::from("."));
        normalize(cwd.join(p))
    }
}

/// POST one tool call to the Godot editor addon bridge.
pub async fn call(tool: &str) -> Result<Value> {
    let url = bridge_url();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building http client")?;
    let mut data: Value = client
        .post(&url)
        .json(&serde_json::json!({"tool": tool}))
        .send()
        .await
        .map_err(|e| anyhow!("Cannot reach Godot editor bridge at {url}: {e}"))?
        .json()
        .await
        .map_err(|e| anyhow!("bad JSON from Godot bridge: {e}"))?;
    if tool == "screenshot" && data.get("error").is_none() {
        let raw_path = data
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("screenshot reply has no path"))?;
        let candidate = absolutize(std::path::Path::new(raw_path));
        let dir = absolutize(&capture_dir());
        let ok_ext = candidate.extension().and_then(|e| e.to_str()) == Some("png");
        let ok_dir = candidate.parent() == Some(dir.as_path());
        if !ok_ext || !ok_dir {
            anyhow::bail!("Screenshot path rejected: outside the MCP capture directory");
        }
        let raw = std::fs::read(&candidate)
            .map_err(|e| anyhow!("cannot read screenshot {}: {e}", candidate.display()))?;
        if raw.len() as u64 > SCREENSHOT_MAX_BYTES {
            anyhow::bail!("Screenshot exceeds the 20 MiB limit");
        }
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &raw);
        if let Some(obj) = data.as_object_mut() {
            obj.insert("_image_b64".to_string(), Value::String(b64));
            obj.insert(
                "_image_path".to_string(),
                Value::String(candidate.to_string_lossy().into_owned()),
            );
        }
    }
    Ok(data)
}
