//! Safe-mode script validator for `execute_blender_code` (BLENDER_MCP_SAFE_MODE).
//! Safe-mode validator: deny-by-default walk over a Python
//! script, parsed with tree-sitter-python. Divergences are documented at the
//! bottom of `validate_code`.

use std::fmt;

pub const SAFE_MODE_ENV: &str = "BLENDER_MCP_SAFE_MODE";

pub fn safe_mode_enabled() -> bool {
    matches!(
        std::env::var(SAFE_MODE_ENV)
            .unwrap_or_default()
            .trim()
            .to_lowercase()
            .as_str(),
        "true" | "1" | "yes" | "on"
    )
}

#[derive(Debug)]
pub struct SandboxViolation {
    pub message: String,
    pub line: Option<usize>,
}

impl fmt::Display for SandboxViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(n) => write!(f, "line {}: {}", n, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for SandboxViolation {}

pub const MAX_CODE_BYTES: usize = 200_000;
pub const MAX_AST_NODES: usize = 20_000;
pub const MAX_NESTING_DEPTH: usize = 24;

static ALLOWED_MODULES: &[&str] = &[
    "bpy",
    "bmesh",
    "mathutils",
    "math",
    "cmath",
    "random",
    "colorsys",
    "json",
    "itertools",
    "functools",
    "collections",
    "statistics",
    "string",
    "re",
    "enum",
    "dataclasses",
    "typing",
    "decimal",
    "fractions",
    "textwrap",
    "unicodedata",
    "uuid",
    "copy",
    "heapq",
    "bisect",
    "array",
];

static DENIED_SUBMODULES: &[&str] = &[
    "bpy.utils.previews",
    "random.SystemRandom",
    "collections.abc",
];

static DENIED_IMPORT_NAMES: &[&str] = &[
    "system",
    "popen",
    "SystemRandom",
    "previews",
    "path",
    "environ",
    "exit",
    "argv",
    "modules",
    "builtins",
    "__builtins__",
    "__import__",
    "__loader__",
    "__spec__",
    "reload",
    "import_module",
    "find_spec",
    "util",
];

static ALLOWED_BUILTINS: &[&str] = &[
    "bool",
    "int",
    "float",
    "complex",
    "str",
    "bytes",
    "bytearray",
    "list",
    "tuple",
    "set",
    "frozenset",
    "dict",
    "slice",
    "abs",
    "round",
    "min",
    "max",
    "sum",
    "pow",
    "divmod",
    "hex",
    "oct",
    "bin",
    "ord",
    "chr",
    "len",
    "range",
    "enumerate",
    "zip",
    "map",
    "filter",
    "reversed",
    "sorted",
    "all",
    "any",
    "iter",
    "next",
    "isinstance",
    "issubclass",
    "callable",
    "repr",
    "format",
    "hash",
    "id",
    "print",
    "type",
    "Exception",
    "ValueError",
    "TypeError",
    "KeyError",
    "IndexError",
    "RuntimeError",
    "AttributeError",
    "ZeroDivisionError",
    "StopIteration",
    "NotImplementedError",
    "ArithmeticError",
    "OverflowError",
    "LookupError",
    "AssertionError",
    "FloatingPointError",
    "True",
    "False",
    "None",
    "NotImplemented",
    "Ellipsis",
];

static FORBIDDEN_CALLABLES: &[(&str, &str)] = &[
    ("eval", "eval() executes arbitrary expressions"),
    ("exec", "exec() executes arbitrary code"),
    ("compile", "compile() produces executable code objects"),
    ("__import__", "__import__() bypasses the import allowlist"),
    (
        "open",
        "open() grants raw filesystem access; use bpy operators for file work",
    ),
    ("input", "input() blocks Blender's main thread on stdin"),
    (
        "breakpoint",
        "breakpoint() drops into a debugger with full process access",
    ),
    ("exit", "exit() terminates the host process"),
    ("quit", "quit() terminates the host process"),
    ("globals", "globals() exposes the module namespace"),
    ("locals", "locals() exposes the enclosing namespace"),
    ("vars", "vars() exposes an object's __dict__"),
    (
        "dir",
        "dir() enumerates attributes for use in dynamic lookups",
    ),
    ("help", "help() imports arbitrary modules via pydoc"),
    ("memoryview", "memoryview() enables raw buffer manipulation"),
    (
        "super",
        "super() reaches base classes that may be blocked types",
    ),
    (
        "object",
        "bare object() is used to reach __subclasses__ chains",
    ),
    (
        "staticmethod",
        "descriptor construction is not needed in scripts",
    ),
    (
        "classmethod",
        "descriptor construction is not needed in scripts",
    ),
    (
        "property",
        "descriptor construction is not needed in scripts",
    ),
    ("copyright", "site builtins expose module internals"),
    ("credits", "site builtins expose module internals"),
    ("license", "site builtins expose module internals"),
];

static LITERAL_ONLY_ATTR_CALLS: &[&str] = &["getattr", "setattr", "delattr", "hasattr"];

static FORBIDDEN_ATTRS: &[&str] = &[
    "__class__",
    "__bases__",
    "__base__",
    "__subclasses__",
    "__mro__",
    "mro",
    "__globals__",
    "__code__",
    "__closure__",
    "__func__",
    "__self__",
    "__builtins__",
    "__dict__",
    "__getattribute__",
    "__getattr__",
    "__setattr__",
    "__delattr__",
    "__reduce__",
    "__reduce_ex__",
    "__init_subclass__",
    "__subclasshook__",
    "__import__",
    "__loader__",
    "__spec__",
    "__package__",
    "__file__",
    "__path__",
    "__module__",
    "__qualname__",
    "__wrapped__",
    "func_globals",
    "func_code",
    "func_closure",
    "gi_frame",
    "cr_frame",
    "f_globals",
    "f_locals",
    "f_builtins",
    "f_back",
    "tb_frame",
    "__objclass__",
    "__weakref__",
    "__annotations__",
    "__defaults__",
    "__kwdefaults__",
    "__new__",
    "__init__",
    "__call__",
    "__getstate__",
    "__setstate__",
    "__sizeof__",
    "__format__",
    "__doc__",
];

static FORBIDDEN_BPY_PATHS: &[(&str, &str)] = &[
    (
        "bpy.app.driver_namespace",
        "driver_namespace injects globals into driver eval",
    ),
    (
        "bpy.app.handlers",
        "handler registration persists code past this script",
    ),
    ("bpy.app.timers", "timers persist code past this script"),
    (
        "bpy.app.binary_path",
        "exposes the Blender executable path for re-launch",
    ),
    (
        "bpy.utils.register_class",
        "class registration persists code past this script",
    ),
    (
        "bpy.utils.unregister_class",
        "class registration persists code past this script",
    ),
    (
        "bpy.utils.register_classes_factory",
        "class registration persists code",
    ),
    ("bpy.utils.execfile", "executes a file from disk"),
    ("bpy.utils.load_scripts", "executes scripts from disk"),
    (
        "bpy.utils.script_paths",
        "enumerates disk locations for script loading",
    ),
    (
        "bpy.utils.user_resource",
        "resolves writable on-disk resource paths",
    ),
    (
        "bpy.utils.modules_from_path",
        "imports arbitrary modules from disk",
    ),
    (
        "bpy.utils.refresh_script_paths",
        "reloads scripts from disk",
    ),
    (
        "bpy.data.texts",
        "text datablocks are an execution path (Run Script)",
    ),
    (
        "bpy.data.scripts",
        "script datablocks are an execution path",
    ),
    (
        "bpy.data.libraries",
        "library loading links external .blend files, which can carry code",
    ),
    (
        "bpy.props",
        "property registration persists definitions past this script",
    ),
    (
        "bpy.types.Operator",
        "defining operators registers persistent code",
    ),
    (
        "bpy.types.Panel",
        "defining panels registers persistent UI code",
    ),
    (
        "bpy.types.AddonPreferences",
        "addon preference classes persist",
    ),
    ("bpy.types.Macro", "macros chain operator execution"),
    (
        "bpy.ops.wm.append",
        "appends datablocks from an external .blend (code can ride along)",
    ),
    (
        "bpy.ops.wm.link",
        "links datablocks from an external .blend (code can ride along)",
    ),
    (
        "bpy.ops.wm.lib_relocate",
        "repoints a library at an arbitrary .blend",
    ),
    ("bpy.ops.wm.lib_reload", "reloads a library from disk"),
    (
        "bpy.ops.wm.save_homefile",
        "overwrites the user's startup file",
    ),
    ("bpy.ops.wm.url_open", "opens a URL in the user's browser"),
    ("bpy.ops.wm.path_open", "opens a path with the OS handler"),
    (
        "bpy.ops.wm.console_toggle",
        "exposes an interactive Python console",
    ),
    (
        "bpy.ops.render.play_rendered_anim",
        "launches an external player process",
    ),
    (
        "bpy.context.preferences.filepaths.script_directories",
        "adds a persistent script search path Blender loads code from",
    ),
];

static FORBIDDEN_OPS_PREFIXES: &[&str] = &[
    "bpy.ops.script",
    "bpy.ops.text",
    "bpy.ops.preferences",
    "bpy.ops.extensions",
    "bpy.ops.console",
];

static MODULE_NAVIGATION: &[&str] = &[
    "ops",
    "utils",
    "app",
    "props",
    "types",
    "wm",
    "script",
    "preferences",
    "extensions",
];

static FORBIDDEN_BARE_ATTRS: &[(&str, &str)] = &[
    (
        "driver_namespace",
        "driver_namespace injects globals into driver expression eval",
    ),
    (
        "register_class",
        "class registration persists code past this script",
    ),
    (
        "unregister_class",
        "class registration persists code past this script",
    ),
    ("execfile", "executes a file from disk"),
    ("load_scripts", "executes scripts from disk"),
    ("save_homefile", "overwrites the user's startup file"),
    ("quit_blender", "terminates the host process"),
    ("url_open", "opens a URL in the user's browser"),
    ("path_open", "opens a path with the OS handler"),
    ("console_toggle", "exposes an interactive Python console"),
    (
        "as_pointer",
        "leaks a raw memory address usable with ctypes",
    ),
    (
        "driver_add",
        "drivers evaluate python expressions on every frame",
    ),
    (
        "driver_remove",
        "driver manipulation is part of the driver eval surface",
    ),
    (
        "handlers",
        "handler registration persists code past this script",
    ),
    ("timers", "timers persist code past this script"),
    (
        "app_handlers",
        "handler registration persists code past this script",
    ),
    (
        "binary_path",
        "exposes the Blender executable path for re-launch",
    ),
    ("user_resource", "resolves writable on-disk resource paths"),
    (
        "script_paths",
        "enumerates disk locations for script loading",
    ),
    (
        "script_directories",
        "adds a persistent script search path Blender loads code from",
    ),
    ("modules_from_path", "imports arbitrary modules from disk"),
    ("python_file_run", "executes a python file from disk"),
    ("run_script", "executes a text datablock as python"),
    ("addon_install", "installs an addon from disk"),
    (
        "addon_enable",
        "enables an addon, executing its module-level code",
    ),
    (
        "texts",
        "text datablocks are an execution path (Run Script)",
    ),
    ("scripts", "script datablocks are an execution path"),
    (
        "libraries",
        "library loading links external .blend files, which can carry code",
    ),
];

static FORBIDDEN_ASSIGN_ATTRS: &[(&str, &str)] = &[
    (
        "expression",
        "driver expressions are evaluated by Blender as python",
    ),
    ("script", "script nodes execute their assigned datablock"),
    (
        "use_self",
        "enables driver expression access to the owning datablock",
    ),
    ("script_directory", "redirects Blender's script search path"),
    (
        "use_scripts_auto_execute",
        "enables automatic execution of embedded scripts",
    ),
];

fn forbidden_callable(name: &str) -> Option<&'static str> {
    FORBIDDEN_CALLABLES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, r)| *r)
}

fn bare_attr_reason(attr: &str) -> Option<&'static str> {
    FORBIDDEN_BARE_ATTRS
        .iter()
        .find(|(n, _)| *n == attr)
        .map(|(_, r)| *r)
}

fn assign_attr_reason(attr: &str) -> Option<&'static str> {
    FORBIDDEN_ASSIGN_ATTRS
        .iter()
        .find(|(n, _)| *n == attr)
        .map(|(_, r)| *r)
}

fn path_blocked(path: &str) -> Option<String> {
    for (prefix, reason) in FORBIDDEN_BPY_PATHS {
        if path == *prefix || path.starts_with(&format!("{prefix}.")) {
            return Some(format!("{path} is forbidden: {reason}"));
        }
    }
    for prefix in FORBIDDEN_OPS_PREFIXES {
        if path == *prefix || path.starts_with(&format!("{prefix}.")) {
            return Some(format!(
                "{path} is forbidden: {prefix}.* executes code or installs addons"
            ));
        }
    }
    None
}

fn is_dunder(s: &str) -> bool {
    s.len() > 4 && s.starts_with("__") && s.ends_with("__")
}

struct Ctx<'a> {
    src: &'a [u8],
    imported: std::collections::HashSet<String>,
    bound: std::collections::HashSet<String>,
    functions: std::collections::HashSet<String>,
    from_imported: std::collections::HashSet<String>,
    nodes: usize,
}

impl<'a> Ctx<'a> {
    fn fail(&self, node: tree_sitter::Node, msg: String) -> Result<(), SandboxViolation> {
        Err(SandboxViolation {
            message: msg,
            line: Some(node.start_position().row + 1),
        })
    }

    fn text(&self, node: tree_sitter::Node) -> String {
        node.utf8_text(self.src).unwrap_or("").to_string()
    }

    fn bind(&mut self, name: &str, node: tree_sitter::Node) -> Result<(), SandboxViolation> {
        if self.imported.contains(name) {
            return self.fail(node, format!("cannot rebind imported module name {name:?}; shadowing modules defeats path-based checks"));
        }
        if forbidden_callable(name).is_some() || LITERAL_ONLY_ATTR_CALLS.contains(&name) {
            return self.fail(node, format!("cannot define {name:?}; rebinding a blocked builtin name is how a script smuggles the real one back in"));
        }
        if ALLOWED_BUILTINS.contains(&name) {
            return self.fail(node, format!("cannot rebind builtin name {name:?}; shadowing makes later calls to it unverifiable"));
        }
        if is_dunder(name) {
            return self.fail(node, format!("cannot bind dunder name {name:?}"));
        }
        self.bound.insert(name.to_string());
        Ok(())
    }

    fn bind_pattern(&mut self, node: tree_sitter::Node) -> Result<(), SandboxViolation> {
        match node.kind() {
            "identifier" => self.bind(&self.text(node), node),
            "tuple_pattern" | "list_pattern" => {
                let mut c = node.walk();
                for ch in node.children(&mut c) {
                    if ch.is_named() {
                        self.bind_pattern(ch)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Resolve `a.b.c` rooted at a plain Name; None when unresolvable.
    fn attr_chain(&self, node: tree_sitter::Node) -> Option<String> {
        let mut parts: Vec<String> = vec![];
        let mut cur = node;
        loop {
            if cur.kind() != "attribute" {
                break;
            }
            let attr = cur.child_by_field_name("attribute")?;
            parts.push(self.text(attr));
            cur = cur.child_by_field_name("object")?;
        }
        if cur.kind() != "identifier" {
            return None;
        }
        // Identifiers inside import machinery do not form chains.
        let p = cur.parent()?;
        if matches!(p.kind(), "dotted_name" | "aliased_import") {
            return None;
        }
        parts.push(self.text(cur));
        parts.reverse();
        Some(parts.join("."))
    }

    fn is_store_target(&self, node: tree_sitter::Node) -> bool {
        // Outermost attribute of an assignment/augmented/annotated/delete target.
        let mut cur = node;
        loop {
            let parent = match cur.parent() {
                Some(p) => p,
                None => return false,
            };
            match parent.kind() {
                "attribute" => {
                    // Inner chain links are loads; only the outermost is the target.
                    let obj = parent.child_by_field_name("object");
                    if obj.map(|o| o.id()) != Some(cur.id()) {
                        return false;
                    }
                    cur = parent;
                }
                "assignment" | "augmented_assignment" => {
                    return parent.child_by_field_name("left").map(|l| l.id()) == Some(cur.id());
                }
                "annotated_assignment" => {
                    // `x.y: T` and `x.y: T = v` both store on the target.
                    return parent.child(0).map(|l| l.id()) == Some(cur.id());
                }
                "delete_statement" => return true,
                _ => return false,
            }
        }
    }

    fn is_outermost_attr(&self, node: tree_sitter::Node) -> bool {
        match node.parent() {
            Some(p) if p.kind() == "attribute" => {
                p.child_by_field_name("object").map(|o| o.id()) != Some(node.id())
            }
            _ => true,
        }
    }

    fn check_attribute(&mut self, node: tree_sitter::Node) -> Result<(), SandboxViolation> {
        let attr_node = node.child_by_field_name("attribute").unwrap();
        let attr = self.text(attr_node);
        let store = self.is_store_target(node);
        if FORBIDDEN_ATTRS.contains(&attr.as_str()) {
            return self.fail(
                node,
                format!("attribute {attr:?} is an interpreter escape path"),
            );
        }
        if is_dunder(&attr) {
            return self.fail(node, format!("dunder attribute {attr:?} is not accessible"));
        }
        if let Some(reason) = bare_attr_reason(&attr) {
            return self.fail(node, format!("{attr:?}: {reason}"));
        }
        if store {
            if let Some(reason) = assign_attr_reason(&attr) {
                return self.fail(node, format!("assigning {attr:?}: {reason}"));
            }
        }
        match self.attr_chain(node) {
            None => {
                if MODULE_NAVIGATION.contains(&attr.as_str()) {
                    return self.fail(node, format!("{attr:?} reached through an unresolvable receiver; module navigation must start from a plain name so it can be checked against the path rules"));
                }
            }
            Some(path) => {
                if let Some(msg) = path_blocked(&path) {
                    return self.fail(node, msg);
                }
                if store && path.starts_with("bpy.types.") {
                    return self.fail(
                        node,
                        format!("assigning to {path} registers a persistent RNA property"),
                    );
                }
                if self.is_outermost_attr(node) {
                    let parts: Vec<&str> = path.split('.').collect();
                    if parts.len() > 1
                        && self.imported.contains(parts[0])
                        && parts[1..].iter().all(|p| MODULE_NAVIGATION.contains(p))
                    {
                        return self.fail(node, format!("{path} is a module namespace and may not be used as a value; continue the path to a concrete attribute or call"));
                    }
                }
            }
        }
        Ok(())
    }

    fn check_call(&mut self, node: tree_sitter::Node) -> Result<(), SandboxViolation> {
        let mut c = node.walk();
        let func = node.children_by_field_name("function", &mut c).next();
        let args_node = node.children_by_field_name("arguments", &mut c).next();
        let func = match func {
            Some(f) => f,
            None => {
                // No function field (should not happen); treat as computed.
                return self.fail(node, "call target must be a name or an attribute; computed call targets cannot be validated".to_string());
            }
        };
        // Positional args (excluding keyword_argument nodes).
        let mut positional: Vec<tree_sitter::Node> = vec![];
        if let Some(a) = args_node {
            let mut ac = a.walk();
            for ch in a.children(&mut ac) {
                if ch.is_named() && ch.kind() != "keyword_argument" {
                    positional.push(ch);
                }
            }
        }
        match func.kind() {
            "identifier" => {
                // Skip import-machinery identifiers.
                if let Some(p) = func.parent() {
                    if matches!(p.kind(), "dotted_name" | "aliased_import") {
                        return Ok(());
                    }
                }
                let name = self.text(func);
                if let Some(reason) = forbidden_callable(&name) {
                    return self.fail(node, format!("{name}() is forbidden: {reason}"));
                }
                if LITERAL_ONLY_ATTR_CALLS.contains(&name.as_str()) {
                    if positional.len() < 2 {
                        return self.fail(
                            node,
                            format!("{name}() requires an explicit literal attribute name"),
                        );
                    }
                    let target = positional[1];
                    if target.kind() != "string" {
                        return self.fail(node, format!("{name}() attribute name must be a literal string, not a computed expression"));
                    }
                    let lit = self.text(target);
                    let attr = lit.trim_matches('"').trim_matches('\'');
                    if FORBIDDEN_ATTRS.contains(&attr) || is_dunder(attr) {
                        return self
                            .fail(node, format!("{name}() targets escape attribute {attr:?}"));
                    }
                    if MODULE_NAVIGATION.contains(&attr) {
                        return self.fail(node, format!("{name}() targets module namespace {attr:?}; namespaces may only be reached through a checkable dotted path"));
                    }
                    if let Some(reason) = bare_attr_reason(attr) {
                        return self.fail(node, format!("{name}() targets {attr:?}: {reason}"));
                    }
                    if (name == "setattr" || name == "delattr")
                        && assign_attr_reason(attr).is_some()
                    {
                        let reason = assign_attr_reason(attr).unwrap();
                        return self.fail(node, format!("{name}() targets {attr:?}: {reason}"));
                    }
                }
                if name == "type" && positional.len() != 1 {
                    return self.fail(
                        node,
                        "type() with three arguments creates a class dynamically".to_string(),
                    );
                } else if !(self.functions.contains(&name)
                    || self.imported.contains(&name)
                    || self.from_imported.contains(&name)
                    || ALLOWED_BUILTINS.contains(&name.as_str())
                    || LITERAL_ONLY_ATTR_CALLS.contains(&name.as_str()))
                {
                    return self.fail(node, format!("{name:?} is not a known callable; call targets must be a def in this script, an allowlisted builtin, or an attribute of an imported module"));
                }
            }
            "attribute" => {}
            _ => {
                return self.fail(node, "call target must be a name or an attribute; computed call targets cannot be validated".to_string());
            }
        }
        Ok(())
    }

    fn check_import(
        &mut self,
        node: tree_sitter::Node,
        from: bool,
    ) -> Result<(), SandboxViolation> {
        // Gather dotted names + alias flags, skipping keywords/punctuation.
        let mut c = node.walk();
        let named: Vec<tree_sitter::Node> =
            node.children(&mut c).filter(|n| n.is_named()).collect();
        if from {
            // from <module> import <names>
            let mut module: Option<String> = None;
            let mut names: Vec<(String, Option<String>)> = vec![]; // (name, asname)
            let mut relative = false;
            let mut wildcard = false;
            for n in &named {
                match n.kind() {
                    "dotted_name" if module.is_none() => module = Some(self.text(*n)),
                    "relative_import" => relative = true,
                    "wildcard_import" => wildcard = true,
                    "dotted_name" => names.push((self.text(*n), None)),
                    "aliased_import" => {
                        let mut ac = n.walk();
                        let parts: Vec<tree_sitter::Node> =
                            n.children(&mut ac).filter(|x| x.is_named()).collect();
                        let nm = parts.first().map(|x| self.text(*x)).unwrap_or_default();
                        let al = parts.get(1).map(|x| self.text(*x));
                        names.push((nm, al));
                    }
                    _ => {}
                }
            }
            if relative {
                return self.fail(node, "relative imports are not allowed".to_string());
            }
            // `from __future__ import ...`
            if node.kind() == "future_import_statement" {
                return self.fail(node, "import from '__future__' is not allowed".to_string());
            }
            let module = module.unwrap_or_default();
            let root = module.split('.').next().unwrap_or("");
            if !ALLOWED_MODULES.contains(&root) || DENIED_SUBMODULES.contains(&module.as_str()) {
                return self.fail(node, format!("import from {module:?} is not allowed"));
            }
            if root == "bpy" {
                return self.fail(node, "from-imports of bpy are not allowed; use `import bpy` and full dotted paths so they can be checked".to_string());
            }
            if wildcard {
                return self.fail(
                    node,
                    "wildcard import hides what enters the namespace".to_string(),
                );
            }
            for (nm, al) in names {
                if DENIED_IMPORT_NAMES.contains(&nm.as_str()) {
                    return self.fail(
                        node,
                        format!("importing {nm:?} from {module:?} is not allowed"),
                    );
                }
                if nm.starts_with('_') {
                    return self.fail(
                        node,
                        format!("importing private name {nm:?} is not allowed"),
                    );
                }
                let bound = al.unwrap_or(nm);
                self.from_imported.insert(bound.clone());
                self.bind(&bound, node)?;
            }
            // from-imports do not add module roots to `imported`.
        } else {
            // import <dotted> [, ...] — aliasing banned.
            for n in &named {
                if n.kind() == "aliased_import" {
                    let mut ac = n.walk();
                    let parts: Vec<tree_sitter::Node> =
                        n.children(&mut ac).filter(|x| x.is_named()).collect();
                    let nm = parts.first().map(|x| self.text(*x)).unwrap_or_default();
                    return self.fail(node, format!("import aliasing ({nm} as ...) is not allowed; it hides the module from path checks"));
                }
                if n.kind() == "dotted_name" {
                    let full = self.text(*n);
                    let root = full.split('.').next().unwrap_or("");
                    if !ALLOWED_MODULES.contains(&root) {
                        return self.fail(node, format!("import of {full:?} is not allowed"));
                    }
                    if DENIED_SUBMODULES.contains(&full.as_str()) {
                        return self.fail(node, format!("import of {full:?} is not allowed"));
                    }
                    self.imported.insert(root.to_string());
                }
            }
        }
        Ok(())
    }
}

fn collect_bindings(
    src: &[u8],
    root: tree_sitter::Node,
    ctx: &mut Ctx,
) -> Result<(), SandboxViolation> {
    // Pre-pass 1: import roots first (shadowing checks depend on them).
    // Pre-pass 2: defs, from-imports, and every other binding.
    let mut stack = vec![root];
    let mut all: Vec<tree_sitter::Node> = vec![];
    while let Some(n) = stack.pop() {
        all.push(n);
        let mut c = n.walk();
        for ch in n.children(&mut c) {
            if ch.is_named() {
                stack.push(ch);
            }
        }
    }
    for n in &all {
        if n.kind() == "import_statement" {
            // Roots only; aliasing is rejected in the main walk.
            let mut c = n.walk();
            for ch in n.children(&mut c) {
                if ch.is_named() && ch.kind() == "dotted_name" {
                    let full = ctx.text(ch);
                    if let Some(root) = full.split('.').next() {
                        ctx.imported.insert(root.to_string());
                    }
                }
            }
        }
    }
    for n in &all {
        match n.kind() {
            "import_from_statement" | "future_import_statement" => {
                let mut c = n.walk();
                for ch in n.children(&mut c) {
                    if !ch.is_named() {
                        continue;
                    }
                    match ch.kind() {
                        "dotted_name" => {
                            // First dotted_name is the module; the rest are names.
                            // Determined by position: skip the first.
                            // (Handled below with index tracking.)
                        }
                        "aliased_import" => {
                            let mut ac = ch.walk();
                            let parts: Vec<tree_sitter::Node> =
                                ch.children(&mut ac).filter(|x| x.is_named()).collect();
                            if let Some(nm) = parts.first() {
                                let nm_t = ctx.text(*nm);
                                let al = parts
                                    .get(1)
                                    .map(|x| ctx.text(*x))
                                    .unwrap_or_else(|| nm_t.clone());
                                if al != "*" {
                                    ctx.from_imported.insert(al.clone());
                                }
                            }
                        }
                        "wildcard_import" => {}
                        _ => {}
                    }
                }
                // Module is the first dotted_name; names follow.
                let mut c2 = n.walk();
                let dotted: Vec<tree_sitter::Node> = n
                    .children(&mut c2)
                    .filter(|x| x.is_named() && x.kind() == "dotted_name")
                    .collect();
                for d in dotted.iter().skip(1) {
                    let nm = ctx.text(*d);
                    ctx.from_imported.insert(nm);
                }
            }
            "function_definition" => {
                if let Some(nm) = n.child_by_field_name("name") {
                    let t = ctx.text(nm);
                    ctx.functions.insert(t);
                }
            }
            _ => {}
        }
    }
    // Every other binding (Store/Del names, args, except aliases, del targets,
    // loop/with targets) — with shadowing rules applied.
    for n in &all {
        // Skip identifiers that are import machinery or keyword names.
        match n.kind() {
            "identifier" => {
                if !is_binding_position(src, *n) {
                    continue;
                }
                let t = ctx.text(*n);
                ctx.bind(&t, *n)?;
            }
            _ => {}
        }
    }
    // Function names + from-imports are bindings too (shadowing rules apply
    // via _bind at their definition sites in the main walk).
    Ok(())
}

/// True when this identifier occurrence binds a name (assignment target,
/// parameter, except alias, del target, loop/with target, def name).
fn is_binding_position(_src: &[u8], node: tree_sitter::Node) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };
    match parent.kind() {
        "dotted_name" | "aliased_import" | "keyword_argument" => return false,
        "attribute" => {
            // The attribute field itself (`obj.attr`); object handled by chain rules.
            if parent.child_by_field_name("attribute").map(|a| a.id()) == Some(node.id()) {
                return false;
            }
        }
        _ => {}
    }
    // Walk up through patterns/left positions to a binding construct.
    let mut cur = node;
    loop {
        let p = match cur.parent() {
            Some(p) => p,
            None => return false,
        };
        match p.kind() {
            "assignment" | "augmented_assignment" => {
                return p.child_by_field_name("left").map(|l| l.id()) == Some(cur.id())
                    || is_pattern_child(p.child_by_field_name("left"), cur);
            }
            "annotated_assignment" => {
                return p.child(0).map(|l| l.id()) == Some(cur.id())
                    || p.child(0)
                        .map(|l| is_pattern_child(Some(l), cur))
                        .unwrap_or(false);
            }
            "for_statement" => {
                return p
                    .child_by_field_name("left")
                    .map(|l| is_pattern_child(Some(l), cur))
                    .unwrap_or(false);
            }
            "for_in_clause" => {
                // Left pattern of the comprehension clause.
                let mut cc = p.walk();
                if let Some(first) = p.children(&mut cc).find(|x| x.is_named()) {
                    if first.id() == cur.id() || is_pattern_child(Some(first), cur) {
                        return true;
                    }
                }
                return false;
            }
            "with_item" => {
                // `with EXPR as NAME`: alias is the last named child.
                let mut cc = p.walk();
                let named: Vec<_> = p.children(&mut cc).filter(|x| x.is_named()).collect();
                return named.last().map(|l| l.id()) == Some(cur.id());
            }
            "except_clause" => {
                let mut cc = p.walk();
                let named: Vec<_> = p.children(&mut cc).filter(|x| x.is_named()).collect();
                return named.last().map(|l| l.id()) == Some(cur.id()) && named.len() > 1;
            }
            "function_definition" => {
                if p.child_by_field_name("name").map(|x| x.id()) == Some(cur.id()) {
                    return true;
                }
                return false;
            }
            "parameters" | "lambda_parameters" => return true,
            "tuple_pattern" | "list_pattern" | "tuple" | "list" => {
                cur = p;
                continue;
            }
            "delete_statement" => return true,
            _ => return false,
        }
    }
}

fn is_pattern_child(pattern: Option<tree_sitter::Node>, target: tree_sitter::Node) -> bool {
    let p = match pattern {
        Some(p) => p,
        None => return false,
    };
    if p.id() == target.id() {
        return true;
    }
    let mut stack = vec![p];
    while let Some(n) = stack.pop() {
        if n.id() == target.id() {
            return true;
        }
        let mut c = n.walk();
        for ch in n.children(&mut c) {
            if ch.is_named() {
                stack.push(ch);
            }
        }
    }
    false
}

/// Validate one script. See module docs for Python divergences.
///
/// Faithful to safe_mode.validate_code: size/NUL/syntax gates, node-count
/// and depth gates, deny-by-default imports/calls/attributes/names, module
/// namespace laundering rules, literal-only getattr family, bpy path rules.
///
/// Known divergences from safe_mode.py:
/// - Structural gate is a denylist of dangerous node kinds (lambda, class,
///   global, nonlocal, walrus, await, yield, match, decorators, type alias,
///   async variants) rather than an allowlist of all 60+ AST kinds; exotic
///   inert syntax may pass that CPython's allowlist would reject.
/// - Nesting depth counts named tree-sitter nodes (operators are anonymous
///   and uncounted), so it is marginally more lenient than ast depth.
/// - Adjacent string literal concatenation ("a" "b") is a
///   concatenated_string node, not a plain string: getattr(x, "a" "b") is
///   rejected here and allowed there.
/// - A bare `import a.b` pre-registers root `a` before the walk, as in
///   Python; shadowing-before-import order is treated as shadowed.
pub fn validate_code(code: &str) -> Result<(), SandboxViolation> {
    let bytes = code.as_bytes();
    if bytes.len() > MAX_CODE_BYTES {
        return Err(SandboxViolation {
            message: format!("script exceeds {MAX_CODE_BYTES} bytes"),
            line: None,
        });
    }
    if bytes.contains(&0) {
        return Err(SandboxViolation {
            message: "script contains a NUL byte".to_string(),
            line: None,
        });
    }
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| SandboxViolation {
            message: format!("validator unavailable: {e}"),
            line: None,
        })?;
    let tree = parser.parse(code, None).ok_or_else(|| SandboxViolation {
        message: "unparseable script".to_string(),
        line: None,
    })?;
    let root = tree.root_node();
    if root.has_error() {
        let line = first_error_line(root);
        return Err(SandboxViolation {
            message: "syntax error: invalid Python".to_string(),
            line,
        });
    }
    let src = bytes;
    let mut ctx = Ctx {
        src,
        imported: Default::default(),
        bound: Default::default(),
        functions: Default::default(),
        from_imported: Default::default(),
        nodes: 0,
    };
    collect_bindings(src, root, &mut ctx)?;
    // Recursive walk; depth is gated at entry so stack use stays bounded.
    walk(&mut ctx, root, 0)?;
    Ok(())
}

fn first_error_line(node: tree_sitter::Node) -> Option<usize> {
    if node.kind() == "ERROR" || node.is_missing() {
        return Some(node.start_position().row + 1);
    }
    let mut c = node.walk();
    for ch in node.children(&mut c) {
        if let Some(l) = first_error_line(ch) {
            return Some(l);
        }
    }
    None
}

fn walk(ctx: &mut Ctx, node: tree_sitter::Node, depth: usize) -> Result<(), SandboxViolation> {
    if depth > MAX_NESTING_DEPTH {
        return Err(SandboxViolation {
            message: format!("script nests deeper than {MAX_NESTING_DEPTH} levels"),
            line: Some(node.start_position().row + 1),
        });
    }
    ctx.nodes += 1;
    if ctx.nodes > MAX_AST_NODES {
        return Err(SandboxViolation {
            message: format!("script exceeds {MAX_AST_NODES} AST nodes"),
            line: Some(node.start_position().row + 1),
        });
    }
    if !node.is_named() {
        return Ok(());
    }
    // Structural denylist (Python bans via allowlist; these are the
    // capability-relevant kinds — see module docs).
    match node.kind() {
        "lambda" => {
            return ctx.fail(
                node,
                "lambdas are indirection that hides the real call target; use def".to_string(),
            )
        }
        "class_definition" => {
            return ctx.fail(
                node,
                "class definitions are the registration path for persistent bpy types".to_string(),
            )
        }
        "global_statement" => {
            return ctx.fail(node, "global rebinds module-scope names".to_string())
        }
        "nonlocal_statement" => {
            return ctx.fail(node, "nonlocal rebinds enclosing-scope names".to_string())
        }
        "named_expression" => {
            return ctx.fail(
                node,
                "walrus assignment obscures call-target analysis".to_string(),
            )
        }
        "await" => {
            return ctx.fail(
                node,
                "async has no event loop in Blender's main thread".to_string(),
            )
        }
        "yield" => {
            return ctx.fail(
                node,
                "generators defer execution past validation".to_string(),
            )
        }
        "match_statement" => {
            return ctx.fail(
                node,
                "structural pattern matching can bind names implicitly".to_string(),
            )
        }
        "decorated_definition" | "decorator" => {
            return ctx.fail(
                node,
                "decorators are not allowed; they apply arbitrary callables to function objects"
                    .to_string(),
            )
        }
        "wildcard_import" | "relative_import" => {
            return ctx.fail(
                node,
                "relative/wildcard imports are not allowed".to_string(),
            )
        }
        "future_import_statement" => {
            return ctx.fail(node, "import from '__future__' is not allowed".to_string())
        }
        "type_alias_statement" => {
            return ctx.fail(node, "TypeAlias is not on the allowlist".to_string())
        }
        "exec_statement" | "print_statement" => {
            return ctx.fail(node, "legacy Python-2 statement is not allowed".to_string())
        }
        _ => {}
    }
    // async def / async for / async with.
    if matches!(
        node.kind(),
        "function_definition" | "for_statement" | "with_statement"
    ) {
        if let Some(first) = node.child(0) {
            if !first.is_named() && ctx.text(first) == "async" {
                return ctx.fail(
                    node,
                    "async has no event loop in Blender's main thread".to_string(),
                );
            }
        }
    }
    match node.kind() {
        "import_statement" => ctx.check_import(node, false)?,
        "import_from_statement" => ctx.check_import(node, true)?,
        "call" => ctx.check_call(node)?,
        "attribute" => ctx.check_attribute(node)?,
        "identifier" => check_name(ctx, node)?,
        "function_definition" => {
            // def name binds (shadowing rules apply).
            if let Some(nm) = node.child_by_field_name("name") {
                let t = ctx.text(nm);
                ctx.bind(&t, nm)?;
            }
        }
        _ => {}
    }
    let mut c = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut c).collect();
    for ch in children {
        // Import machinery is fully handled by check_import; do not walk into it.
        if matches!(
            node.kind(),
            "import_statement" | "import_from_statement" | "future_import_statement"
        ) {
            continue;
        }
        // Function name/parameters handled via bind rules; walk body only.
        // (Parameters bind through is_binding_position in the pre-pass; the
        // main walk still visits them for _bind shadowing checks.)
        walk(ctx, ch, depth + 1)?;
        let _ = ch;
    }
    Ok(())
}

fn check_name(ctx: &mut Ctx, node: tree_sitter::Node) -> Result<(), SandboxViolation> {
    // Skip import machinery, attribute fields, keyword names, def names —
    // those are handled by their constructs, not as name loads.
    if let Some(p) = node.parent() {
        match p.kind() {
            "dotted_name" | "aliased_import" => return Ok(()),
            "keyword_argument" => {
                // `f(name=1)`: the keyword name itself is not a value.
                if p.child_by_field_name("name").map(|n| n.id()) == Some(node.id()) {
                    return Ok(());
                }
            }
            "attribute" => {
                if p.child_by_field_name("attribute").map(|a| a.id()) == Some(node.id()) {
                    return Ok(());
                }
                // Object position falls through to chain/laundering rules below.
            }
            "function_definition" => {
                if p.child_by_field_name("name").map(|n| n.id()) == Some(node.id()) {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
    let name = ctx.text(node);
    // Store/Del positions bind (shadowing rules apply).
    if is_binding_position(ctx.src, node) {
        return ctx.bind(&name, node).map(|_| ());
    }
    // Load position.
    if forbidden_callable(&name).is_some() {
        let reason = forbidden_callable(&name).unwrap();
        return ctx.fail(node, format!("{name} is forbidden: {reason}"));
    }
    if is_dunder(&name) {
        return ctx.fail(node, format!("dunder name {name:?} is not accessible"));
    }
    // Module-namespace laundering: an imported module may only appear as the
    // root of a dotted path.
    if ctx.imported.contains(&name) {
        let chained = match node.parent() {
            Some(p) if p.kind() == "attribute" => {
                p.child_by_field_name("object").map(|o| o.id()) == Some(node.id())
            }
            _ => false,
        };
        if !chained {
            return ctx.fail(node, format!("module {name:?} may only be used as the start of a dotted path; passing the module object around defeats path checks"));
        }
        return Ok(());
    }
    if ctx.bound.contains(&name)
        || ALLOWED_BUILTINS.contains(&name.as_str())
        || LITERAL_ONLY_ATTR_CALLS.contains(&name.as_str())
    {
        return Ok(());
    }
    ctx.fail(node, format!("unknown name {name:?}; only allowlisted builtins, imported modules, and names bound in this script may be used"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benign_bpy_script_passes() {
        assert!(validate_code("import bpy\nbpy.ops.mesh.primitive_cube_add(size=2.0)\n").is_ok());
        assert!(validate_code("import math\nx = math.pi * 2\n").is_ok());
        // Rendering, saving, and import/export operators are allowed.
        assert!(validate_code(
            "import bpy\nbpy.ops.wm.save_as_mainfile(filepath='/tmp/x.blend')\n"
        )
        .is_ok());
    }

    #[test]
    fn hostile_imports_and_io_fail() {
        for code in [
            "import os\n",
            "import sys\n",
            "from os import path\n",
            "open('x').read()\n",
            "eval('1')\n",
        ] {
            let err = validate_code(code).expect_err(&format!("should reject {code:?}"));
            assert!(!err.to_string().is_empty());
        }
    }
}
