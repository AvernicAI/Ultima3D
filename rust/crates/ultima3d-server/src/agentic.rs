//! Agentic pipeline helpers: pure logic over backend payloads, no I/O.
//! Mirrors ultima3d/agentic.py. review_gate additionally accepts the
//! list-of-{check,pass} form the worker actually returns (the Python
//! version only handled the dict form).

use serde_json::Value;

/// Merge per-backend status payloads into one director-facing snapshot.
/// The worker is the only hard dependency.
pub fn combine_status(ultima: Value, godot: Value, live: Value) -> Value {
    let ok = ultima.get("ok").and_then(Value::as_bool).unwrap_or(false);
    serde_json::json!({"ok": ok, "ultima3d": ultima, "godot": godot, "live_blender": live})
}

/// Find the main GLB path inside a finalize_asset files dict.
pub fn pick_export_source(files: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    for key in [format!("{name}.glb"), "main".to_string(), "glb".to_string()] {
        if let Some(Value::String(s)) = files.get(&key) {
            return Some(s.clone());
        }
    }
    for value in files.values() {
        if let Value::String(s) = value {
            if s.ends_with(".glb") && !s.contains("_LOD") && !s.contains("_collision") {
                return Some(s.clone());
            }
        }
    }
    None
}

/// Decide done vs fix from machine validation + critic output.
pub fn review_gate(validation_json: &str, assertions_json: Option<&str>) -> Value {
    let validation: Value = serde_json::from_str(validation_json).unwrap_or(Value::Null);
    let mut failed: Vec<String> = vec![];
    match validation.get("checks") {
        // Worker form: [{"check": ..., "pass": bool, ...}]
        Some(Value::Array(checks)) => {
            for c in checks {
                let pass = c.get("pass").and_then(Value::as_bool).unwrap_or(true);
                if !pass {
                    failed.push(
                        c.get("check").and_then(Value::as_str).unwrap_or("?").to_string(),
                    );
                }
            }
        }
        // Dict form: {"has_uv": true}
        Some(Value::Object(checks)) => {
            for (k, v) in checks {
                if v == &Value::Bool(false) {
                    failed.push(k.clone());
                }
            }
        }
        _ => {}
    }
    failed.sort();
    let mut critic_failed: Vec<Value> = vec![];
    let mut critic_manual: Vec<Value> = vec![];
    if let Some(text) = assertions_json {
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            let results = parsed.get("results").unwrap_or(&parsed);
            if let Some(list) = results.as_array() {
                for entry in list {
                    match entry.get("status").and_then(Value::as_str) {
                        Some("fail") => critic_failed.push(entry.clone()),
                        Some("manual") => critic_manual.push(entry.clone()),
                        _ => {}
                    }
                }
            }
        }
    }
    let pass = validation.get("pass").and_then(Value::as_bool).unwrap_or(true);
    let done = failed.is_empty() && critic_failed.is_empty() && pass;
    let mut fixes: Vec<String> = failed.iter().map(|k| format!("validate:{k}")).collect();
    for e in &critic_failed {
        fixes.push(format!(
            "critic:{}:{}",
            e.get("check").and_then(Value::as_str).unwrap_or("?"),
            e.get("name").and_then(Value::as_str).unwrap_or("")
        ));
    }
    serde_json::json!({
        "done": done,
        "failed_checks": failed,
        "critic_failures": critic_failed,
        "critic_manual": critic_manual,
        "next_fixes": fixes,
    })
}
