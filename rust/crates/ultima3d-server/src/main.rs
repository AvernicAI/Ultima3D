//! Ultima3D MCP server (Rust host). Mirrors ultima3d/server.py:
//! one server, four backends — procedural worker, live Blender GUI,
//! Godot editor bridge, Photoshop CS3 tools.
//!
//! The Blender geometry worker itself stays Python: it runs inside
//! Blender's embedded interpreter, which exposes no Rust API.

mod agentic;
mod godot;
mod photoshop;
mod textures;
mod worker;

use std::env;
use std::path::PathBuf;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

use worker::Worker;

fn env_path(name: &str, fallback: String) -> PathBuf {
    PathBuf::from(env::var(name).unwrap_or(fallback))
}

fn out_dir(sub: &str) -> PathBuf {
    let base = env_path(
        "ULTIMA3D_OUT",
        std::env::current_dir()
            .unwrap_or(PathBuf::from("."))
            .join("output")
            .to_string_lossy()
            .into_owned(),
    );
    let p = if sub.is_empty() { base } else { base.join(sub) };
    let _ = std::fs::create_dir_all(&p);
    p
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Create3dArgs {
    recipe: Map<String, Value>,
    #[serde(default = "default_name")]
    name: String,
    #[serde(default)]
    blueprint: Option<Map<String, Value>>,
    #[serde(default = "default_true")]
    join: bool,
}

fn default_name() -> String {
    "asset".to_string()
}

fn default_true() -> bool {
    true
}

fn default_resolution() -> u32 {
    512
}

fn default_budget() -> i64 {
    8000
}

fn default_tex_scale() -> f64 {
    3.0
}

fn default_tex_blend() -> f64 {
    0.5
}

fn default_collision() -> String {
    "auto".to_string()
}

fn default_tex_res() -> i64 {
    1024
}

fn ok_text(s: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(s)]))
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BlueprintArgs {
    blueprint: Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RefineAssetArgs {
    #[serde(default)]
    changes: Option<Map<String, Value>>,
    #[serde(default)]
    recipe: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RenderAssetArgs {
    #[serde(default)]
    views: Option<Vec<String>>,
    #[serde(default = "default_resolution")]
    resolution: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetMaterialArgs {
    name: String,
    #[serde(default)]
    color: Option<Vec<f64>>,
    #[serde(default)]
    roughness: Option<f64>,
    #[serde(default)]
    metallic: Option<f64>,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    texture: Option<String>,
    #[serde(default = "default_tex_scale")]
    tex_scale: f64,
    #[serde(default = "default_tex_blend")]
    tex_blend: f64,
    #[serde(default)]
    tex_seed: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetGeometryArgs {
    builder: String,
    node_name: String,
    #[serde(default)]
    params: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BudgetArgs {
    #[serde(default = "default_budget")]
    triangle_budget: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AssertionsArgs {
    assertions: Vec<Value>,
    #[serde(default)]
    materials: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FinalizeArgs {
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "default_budget")]
    triangle_budget: i64,
    #[serde(default)]
    lods: Option<Vec<i64>>,
    #[serde(default = "default_collision")]
    collision: String,
    #[serde(default = "default_tex_res")]
    texture_resolution: i64,
    #[serde(default)]
    channels: Option<Vec<String>>,
    #[serde(default = "default_true")]
    bake: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RigArgs {
    #[serde(default)]
    bones: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PathArgs {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LoadRecipeArgs {
    path: String,
    #[serde(default = "default_true")]
    rebuild: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExportGodotArgs {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    dest_dir: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExecArgs {
    code: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateImageArgs {
    #[serde(default = "default_image_size")]
    width: i64,
    #[serde(default = "default_image_size")]
    height: i64,
    #[serde(default = "default_background")]
    background: String,
    #[serde(default = "default_name")]
    name: String,
}

fn default_image_size() -> i64 {
    1024
}

fn default_background() -> String {
    "white".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScriptArgs {
    script: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MakeTextureArgs {
    preset: String,
    #[serde(default = "default_texture_size")]
    size: i64,
    #[serde(default)]
    seed: i64,
}

fn default_texture_size() -> i64 {
    256
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReviewArgs {
    #[serde(default = "default_budget")]
    triangle_budget: i64,
    #[serde(default)]
    assertions: Option<Vec<Value>>,
    #[serde(default)]
    materials: Option<Map<String, Value>>,
}

#[derive(Clone)]
struct Ultima3d {
    tool_router: ToolRouter<Self>,
    worker: Worker,
}

#[tool_router]
impl Ultima3d {
    fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or(PathBuf::from("."));
        let blender = env_path(
            "ULTIMA3D_BLENDER",
            r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe".to_string(),
        );
        let worker_py = env::var("ULTIMA3D_WORKER")
            .map(PathBuf::from)
            .unwrap_or_else(|_| cwd.join("ultima3d").join("blender_worker.py"));
        let timeout: u64 = env::var("ULTIMA3D_CALL_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        let worker = Worker::new(blender, worker_py, timeout);
        Self { tool_router: Self::tool_router(), worker }
    }

    fn err_text(e: anyhow::Error) -> Result<CallToolResult, McpError> {
        // Tool ran and failed: tool-level error content, not a protocol error,
        // so the model receives the message (mirrors ToolError in server.py).
        Ok(CallToolResult::error(vec![rmcp::model::ContentBlock::text(
            e.to_string(),
        )]))
    }

    #[tool(description = "Build a 3D asset in Blender from a parametric recipe and (optionally) a design blueprint.")]
    async fn create_3d(&self, Parameters(args): Parameters<Create3dArgs>) -> Result<CallToolResult, McpError> {
        let params = serde_json::json!({
            "recipe": args.recipe,
            "blueprint": args.blueprint,
            "name": args.name,
            "join": args.join,
        });
        match self.worker.call("build", params).await {
            Ok(r) => Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                r.to_string(),
            )])),
            Err(e) => Self::err_text(e),
        }
    }

    #[tool(description = "Return machine-readable facts about the current asset: triangles, dimensions, materials, UVs, recipe.")]
    async fn inspect_asset(&self) -> Result<CallToolResult, McpError> {
        match self.worker.call("inspect", Value::Object(Default::default())).await {
            Ok(r) => Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                r.to_string(),
            )])),
            Err(e) => Self::err_text(e),
        }
    }

    #[tool(description = "Check the Blender worker: version, liveness, current scene state.")]
    async fn ultima3d_status(&self) -> Result<CallToolResult, McpError> {
        match self.worker.call("ping", Value::Object(Default::default())).await {
            Ok(ping) => {
                let body = serde_json::json!({"ok": true, "blender": ping.get("blender"), "worker": "running"});
                Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                    body.to_string(),
                )]))
            }
            Err(e) => Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::json!({"ok": false, "error": e.to_string()}).to_string(),
            )])),
        }
    }

    #[tool(description = "Clear the Blender scene and forget the current recipe/blueprint.")]
    async fn reset_scene(&self) -> Result<CallToolResult, McpError> {
        self.worker_text("clear", Value::Object(Default::default())).await
    }

    async fn worker_text(&self, op: &str, params: Value) -> Result<CallToolResult, McpError> {
        match self.worker.call(op, params).await {
            Ok(r) => ok_text(r.to_string()),
            Err(e) => Self::err_text(e),
        }
    }

    async fn worker_inspect(&self) -> Value {
        self.worker
            .call("inspect", Value::Object(Default::default()))
            .await
            .unwrap_or(Value::Null)
    }

    #[tool(description = "Validate a v1 asset blueprint and compile it to recipe nodes plus warnings. Feed the recipe to create_3d.")]
    async fn compile_blueprint(
        &self,
        Parameters(args): Parameters<BlueprintArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.worker_text(
            "compile_blueprint",
            serde_json::json!({"blueprint": args.blueprint}),
        )
        .await
    }

    #[tool(description = "Parametric refinement: pass {changes: {node.param: value}} using dotted node names, or {recipe: {...}} to replace the whole recipe.")]
    async fn refine_asset(
        &self,
        Parameters(args): Parameters<RefineAssetArgs>,
    ) -> Result<CallToolResult, McpError> {
        let insp = self.worker_inspect().await;
        let mut last_recipe: Map<String, Value> = insp
            .get("recipe")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let name = insp
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("asset")
            .to_string();
        let blueprint = insp.get("blueprint").cloned().unwrap_or(Value::Null);
        if let Some(recipe) = args.recipe {
            return self
                .worker_text(
                    "build",
                    serde_json::json!({"recipe": recipe, "name": name,
                                       "blueprint": blueprint, "join": true}),
                )
                .await;
        }
        let mut applied: Vec<String> = vec![];
        let mut ignored: Vec<String> = vec![];
        if let Some(changes) = args.changes {
            for (dotted, value) in changes {
                match dotted.split_once('.') {
                    Some((node, key)) if last_recipe.contains_key(node) && !key.is_empty() => {
                        match last_recipe.get_mut(node).and_then(|v| v.as_object_mut()) {
                            Some(obj) => {
                                obj.insert(key.to_string(), value);
                                applied.push(dotted);
                            }
                            None => ignored.push(dotted),
                        }
                    }
                    _ => ignored.push(dotted),
                }
            }
        }
        let mut result = match self
            .worker
            .call(
                "build",
                serde_json::json!({"recipe": last_recipe, "name": name,
                                   "blueprint": blueprint, "join": true}),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return Self::err_text(e),
        };
        let mut dropped: Vec<String> = vec![];
        if let Some(obj) = result.get("ignored_params").and_then(|v| v.as_object()) {
            for (node, info) in obj {
                if let Some(list) = info.get("ignored").and_then(|v| v.as_array()) {
                    for key in list.iter().filter_map(Value::as_str) {
                        dropped.push(format!("{node}.{key}"));
                    }
                }
            }
        }
        dropped.sort();
        if let Some(obj) = result.as_object_mut() {
            let kept: Vec<&String> = applied.iter().filter(|d| !dropped.contains(d)).collect();
            obj.insert("applied".to_string(), serde_json::to_value(&kept).unwrap());
            ignored.extend(dropped);
            obj.insert("ignored".to_string(), serde_json::to_value(&ignored).unwrap());
        }
        ok_text(result.to_string())
    }

    #[tool(description = "Render the asset from standard angles for visual evaluation. Returns the images inline.")]
    async fn render_asset(
        &self,
        Parameters(args): Parameters<RenderAssetArgs>,
    ) -> Result<CallToolResult, McpError> {
        let fallback = ["front", "back", "left", "right", "front_34", "back_34", "top", "wire"];
        let views: Vec<String> = args.views.clone().unwrap_or_else(|| {
            fallback.iter().map(|s| s.to_string()).collect()
        });
        let dir = out_dir("renders");
        let r = match self
            .worker
            .call(
                "render_views",
                serde_json::json!({"views": args.views, "resolution": args.resolution,
                                   "dir": dir.to_string_lossy()}),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return Self::err_text(e),
        };
        use base64::Engine as _;
        let mut blocks = vec![];
        for v in &views {
            let p = r
                .get("renders")
                .and_then(|m| m.get(v))
                .and_then(Value::as_str)
                .unwrap_or("");
            if p.is_empty() {
                continue;
            }
            if let Ok(bytes) = tokio::fs::read(p).await {
                blocks.push(ContentBlock::image(
                    base64::engine::general_purpose::STANDARD.encode(&bytes),
                    "image/png",
                ));
            }
        }
        Ok(CallToolResult::success(blocks))
    }

    #[tool(description = "Apply a PBR material to the asset (or a named object). color = [r,g,b] 0-1. texture names a procedural preset.")]
    async fn set_material(
        &self,
        Parameters(args): Parameters<SetMaterialArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.worker_text(
            "set_material",
            serde_json::json!({
                "name": args.name, "color": args.color, "roughness": args.roughness,
                "metallic": args.metallic, "object": args.object, "texture": args.texture,
                "tex_scale": args.tex_scale, "tex_blend": args.tex_blend,
                "tex_seed": args.tex_seed,
            }),
        )
        .await
    }

    #[tool(description = "Add one node with a given builder to the current recipe and rebuild (additive edit).")]
    async fn set_geometry(
        &self,
        Parameters(args): Parameters<SetGeometryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let insp = self.worker_inspect().await;
        let mut recipe: Map<String, Value> = insp
            .get("recipe")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let mut node = Map::new();
        node.insert("builder".to_string(), Value::String(args.builder));
        if let Some(params) = args.params {
            for (k, v) in params {
                node.insert(k, v);
            }
        }
        recipe.insert(args.node_name, Value::Object(node));
        self.worker_text(
            "build",
            serde_json::json!({
                "recipe": recipe,
                "name": insp.get("name").and_then(Value::as_str).unwrap_or("asset"),
                "blueprint": insp.get("blueprint").cloned().unwrap_or(Value::Null),
                "join": true,
            }),
        )
        .await
    }

    #[tool(description = "Machine-checked validation: duplicates, zero-area faces, budget, UVs, materials, scale, manifoldness.")]
    async fn validate_asset(
        &self,
        Parameters(args): Parameters<BudgetArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.worker_text(
            "validate",
            serde_json::json!({"triangle_budget": args.triangle_budget}),
        )
        .await
    }

    #[tool(description = "Verify blueprint assertions against the live asset: part count, ratios, symmetry, color coverage. Unmeasurable checks return manual.")]
    async fn check_assertions(
        &self,
        Parameters(args): Parameters<AssertionsArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.worker_text(
            "check_assertions",
            serde_json::json!({"assertions": args.assertions, "materials": args.materials}),
        )
        .await
    }

    #[tool(description = "Game-ready export: clean, UV, decimate to budget, bake textures, LODs, collision hull, GLB + asset.json.")]
    async fn finalize_asset(
        &self,
        Parameters(args): Parameters<FinalizeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let channels = args.channels.unwrap_or_else(|| {
            ["albedo", "normal", "roughness"].iter().map(|s| s.to_string()).collect()
        });
        self.worker_text(
            "finalize",
            serde_json::json!({
                "name": args.name, "triangle_budget": args.triangle_budget,
                "lods": args.lods, "collision": args.collision,
                "texture_resolution": args.texture_resolution,
                "channels": channels, "bake": args.bake,
                "dir": out_dir("export").to_string_lossy(),
            }),
        )
        .await
    }

    #[tool(description = "Multi-bone auto-rig: mass-adaptive vertical bone chain with automatic vertex weights.")]
    async fn rig_asset(&self, Parameters(args): Parameters<RigArgs>) -> Result<CallToolResult, McpError> {
        self.worker_text("rig", serde_json::json!({"bones": args.bones})).await
    }

    #[tool(description = "Save the current parametric recipe + blueprint as JSON for later editing/rebuilding.")]
    async fn save_recipe(
        &self,
        Parameters(args): Parameters<PathArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.worker_text("save_recipe", serde_json::json!({"path": args.path})).await
    }

    #[tool(description = "Load a saved recipe; optionally rebuild the asset from it immediately.")]
    async fn load_recipe(
        &self,
        Parameters(args): Parameters<LoadRecipeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let mut r = match self
            .worker
            .call("load_recipe", serde_json::json!({"path": args.path}))
            .await
        {
            Ok(r) => r,
            Err(e) => return Self::err_text(e),
        };
        if args.rebuild {
            let text = match tokio::fs::read_to_string(&args.path).await {
                Ok(t) => t,
                Err(e) => return Self::err_text(anyhow::anyhow!(e)),
            };
            let data: Value = match serde_json::from_str(&text) {
                Ok(d) => d,
                Err(e) => return Self::err_text(anyhow::anyhow!(e)),
            };
            let build = self
                .worker
                .call(
                    "build",
                    serde_json::json!({
                        "recipe": data.get("recipe").cloned().unwrap_or(Value::Null),
                        "name": data.get("asset").and_then(Value::as_str).unwrap_or("asset"),
                        "blueprint": data.get("blueprint").cloned().unwrap_or(Value::Null),
                        "join": true,
                    }),
                )
                .await;
            match build {
                Ok(b) => {
                    if let Some(obj) = r.as_object_mut() {
                        obj.insert("build".to_string(), b);
                    }
                }
                Err(e) => return Self::err_text(e),
            }
        }
        ok_text(r.to_string())
    }

    #[tool(description = "Copy the finalized GLB into a Godot project folder for import.")]
    async fn export_asset_to_godot(
        &self,
        Parameters(args): Parameters<ExportGodotArgs>,
    ) -> Result<CallToolResult, McpError> {
        let dest = args.dest_dir.map(PathBuf::from).unwrap_or_else(|| out_dir("godot_import"));
        if let Err(e) = std::fs::create_dir_all(&dest) {
            return Self::err_text(anyhow::anyhow!(e));
        }
        let mut src_name = args.name;
        if src_name.is_none() {
            let insp = self.worker_inspect().await;
            src_name = insp
                .get("name")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
        }
        let src_name = src_name.unwrap_or_else(|| "asset".to_string());
        let src = out_dir("export").join(format!("{src_name}.glb"));
        if !src.is_file() {
            return Self::err_text(anyhow::anyhow!(
                "no finalized GLB at {}; run finalize_asset(name={src_name:?}) first",
                src.display()
            ));
        }
        let out = dest.join(format!("{src_name}.glb"));
        if let Err(e) = std::fs::copy(&src, &out) {
            return Self::err_text(anyhow::anyhow!(e));
        }
        ok_text(
            serde_json::json!({"ok": true, "src": src.to_string_lossy(),
                               "dest": out.to_string_lossy(),
                               "note": "import it in Godot (auto .godot import on scan); no .tscn is created"})
            .to_string(),
        )
    }

    #[tool(description = "Run validate (+ optional critic) and report done vs next fixes.")]
    async fn review_asset_loop(
        &self,
        Parameters(args): Parameters<ReviewArgs>,
    ) -> Result<CallToolResult, McpError> {
        let validation = match self
            .worker
            .call("validate", serde_json::json!({"triangle_budget": args.triangle_budget}))
            .await
        {
            Ok(v) => v,
            Err(e) => return Self::err_text(e),
        };
        let assertions_json = match args.assertions {
            Some(a) => match self
                .worker
                .call(
                    "check_assertions",
                    serde_json::json!({"assertions": a, "materials": args.materials}),
                )
                .await
            {
                Ok(r) => Some(r.to_string()),
                Err(e) => return Self::err_text(e),
            },
            None => None,
        };
        ok_text(agentic::review_gate(&validation.to_string(), assertions_json.as_deref()).to_string())
    }

    // -- Godot editor bridge --------------------------------------------
    async fn godot_text(&self, tool: &str) -> Result<CallToolResult, McpError> {
        match godot::call(tool).await {
            Ok(r) => ok_text(r.to_string()),
            Err(e) => ok_text(serde_json::json!({"ok": false, "error": e.to_string()}).to_string()),
        }
    }

    #[tool(description = "Get the running Godot editor/project status (offline-tolerant).")]
    async fn godot_status(&self) -> Result<CallToolResult, McpError> {
        self.godot_text("status").await
    }

    #[tool(description = "Inspect the currently edited Godot scene tree (offline-tolerant).")]
    async fn godot_scene_tree(&self) -> Result<CallToolResult, McpError> {
        self.godot_text("scene_tree").await
    }

    #[tool(description = "Run the current Godot project in the editor.")]
    async fn godot_run_project(&self) -> Result<CallToolResult, McpError> {
        self.godot_text("run_project").await
    }

    #[tool(description = "Stop the currently running Godot game.")]
    async fn godot_stop_project(&self) -> Result<CallToolResult, McpError> {
        self.godot_text("stop_project").await
    }

    #[tool(description = "Read buffered Godot editor/game output since the last read.")]
    async fn godot_debug_output(&self) -> Result<CallToolResult, McpError> {
        self.godot_text("debug_output").await
    }

    #[tool(description = "Capture the Godot editor/game viewport as PNG.")]
    async fn godot_screenshot(&self) -> Result<CallToolResult, McpError> {
        use base64::Engine as _;
        match godot::call("screenshot").await {
            Ok(data) => {
                if let Some(err) = data.get("error").and_then(Value::as_str) {
                    return Self::err_text(anyhow::anyhow!(err.to_string()));
                }
                let b64 = data.get("_image_b64").and_then(Value::as_str).unwrap_or("");
                match base64::engine::general_purpose::STANDARD.decode(b64) {
                    Ok(bytes) => Ok(CallToolResult::success(vec![ContentBlock::image(
                        base64::engine::general_purpose::STANDARD.encode(&bytes),
                        "image/png",
                    )])),
                    Err(e) => Self::err_text(anyhow::anyhow!(e)),
                }
            }
            Err(e) => Self::err_text(e),
        }
    }

    // -- Photoshop CS3 ----------------------------------------------------
    #[tool(description = "Check the Photoshop CS3 backend: exe presence, COM reachability, JSX presets.")]
    async fn photoshop_status(&self) -> Result<CallToolResult, McpError> {
        ok_text(photoshop::status().to_string())
    }

    #[tool(description = "Run ExtendScript in the OPEN Photoshop and return its result (live control). Send __quit__ to stop the bridge.")]
    async fn photoshop_exec(
        &self,
        Parameters(args): Parameters<ExecArgs>,
    ) -> Result<CallToolResult, McpError> {
        let code = args.code.trim().to_string();
        let r = if code == "__quit__" {
            photoshop::call(None, "quit", std::time::Duration::from_secs(60))
        } else {
            photoshop::call(Some(&code), "exec", std::time::Duration::from_secs(60))
        };
        match r {
            Ok(v) => ok_text(v.to_string()),
            Err(e) => ok_text(serde_json::json!({"ok": false, "error": e.to_string()}).to_string()),
        }
    }

    #[tool(description = "Create a new RGB image in the open Photoshop (white/transparent/background).")]
    async fn photoshop_create_image(
        &self,
        Parameters(args): Parameters<CreateImageArgs>,
    ) -> Result<CallToolResult, McpError> {
        let code = match photoshop::build_create(args.width, args.height, &args.background, &args.name) {
            Ok(c) => c,
            Err(e) => return ok_text(serde_json::json!({"ok": false, "error": e.to_string()}).to_string()),
        };
        match photoshop::call(Some(&code), "exec", std::time::Duration::from_secs(60)) {
            Ok(v) => ok_text(v.to_string()),
            Err(e) => ok_text(serde_json::json!({"ok": false, "error": e.to_string()}).to_string()),
        }
    }

    #[tool(description = "Run a JSX preset (prep_texture, export_png) or inline ExtendScript in CS3. Fire-and-forget.")]
    async fn photoshop_run_script(
        &self,
        Parameters(args): Parameters<ScriptArgs>,
    ) -> Result<CallToolResult, McpError> {
        match photoshop::run_script(&args.script) {
            Ok(v) => ok_text(v.to_string()),
            Err(e) => ok_text(serde_json::json!({"ok": false, "error": e.to_string()}).to_string()),
        }
    }

    #[tool(description = "Synthesize a tileable texture trio (albedo/rough/normal PNGs) headlessly. Presets: stone, wood_grain, noise.")]
    async fn photoshop_make_texture(
        &self,
        Parameters(args): Parameters<MakeTextureArgs>,
    ) -> Result<CallToolResult, McpError> {
        let dir = out_dir("textures");
        match textures::make_texture(&args.preset, args.size.max(0) as usize, args.seed as u64, &dir, None) {
            Ok(v) => ok_text(v.to_string()),
            Err(e) => ok_text(
                serde_json::json!({"ok": false, "error": e.to_string(),
                                   "presets": textures::PRESETS})
                .to_string(),
            ),
        }
    }
}

#[tool_handler]
impl ServerHandler for Ultima3d {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = Ultima3d::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
