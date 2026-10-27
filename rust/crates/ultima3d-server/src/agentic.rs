//! Agentic pipeline helpers: pure logic over backend payloads, no I/O.
//! review_gate accepts both the dict form and the list-of-{check,pass}
//! form the worker actually returns.

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
                        c.get("check")
                            .and_then(Value::as_str)
                            .unwrap_or("?")
                            .to_string(),
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
    let pass = validation
        .get("pass")
        .and_then(Value::as_bool)
        .unwrap_or(true);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_status_worker_is_the_hard_dependency() {
        let ok = combine_status(
            serde_json::json!({"ok": true, "worker": "running"}),
            serde_json::json!({"ok": false, "error": "no godot"}),
            serde_json::json!({"ok": true}),
        );
        assert_eq!(ok["ok"], true);
        let bad = combine_status(
            serde_json::json!({"ok": false, "error": "no worker"}),
            serde_json::json!({"ok": true}),
            serde_json::json!({"ok": true}),
        );
        assert_eq!(bad["ok"], false);
    }

    #[test]
    fn review_gate_done_vs_fixes() {
        let v = review_gate(r#"{"pass": true, "checks": {"has_uv": true}}"#, None);
        assert_eq!(v["done"], true);
        let v = review_gate(
            r#"{"pass": true, "checks": [{"check": "has_uv", "pass": false}]}"#,
            Some(
                r#"{"results": [{"check": "ratio", "name": "x", "status": "fail"}, {"check": "sym", "status": "manual"}]}"#,
            ),
        );
        assert_eq!(v["done"], false);
        assert_eq!(v["failed_checks"], serde_json::json!(["has_uv"]));
        assert_eq!(v["critic_failures"].as_array().unwrap().len(), 1);
        assert_eq!(v["critic_manual"].as_array().unwrap().len(), 1);
        let fixes = v["next_fixes"].as_array().unwrap();
        assert!(fixes.iter().any(|f| f == "validate:has_uv"));
    }

    #[test]
    fn pick_export_source_prefers_main_glb() {
        let mut files = serde_json::Map::new();
        files.insert("chair_LOD1.glb".into(), serde_json::json!("a"));
        files.insert("chair.glb".into(), serde_json::json!("b"));
        assert_eq!(pick_export_source(&files, "chair").as_deref(), Some("b"));
    }
}
