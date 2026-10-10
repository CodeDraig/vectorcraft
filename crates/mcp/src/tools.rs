//! Execute MCP tools through the engine or connected desktop backend.

mod drawing;
use drawing::{add_text, apply_paint, draw_path, draw_shape, select_ids, transform};
mod capture;
use capture::{render_preview, screenshot};

mod schema;
pub use schema::tool_definitions;

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::ARTBOARD_PARAMS;

use crate::backend::Backend;

/// The result of `tools/call`: MCP content blocks plus the `isError` flag.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: false }
    }
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_default())
    }
    pub fn error(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: true }
    }
    pub fn to_value(&self) -> Value {
        json!({"content": self.content, "isError": self.is_error})
    }
}

// ---------- implementation ----------

type Args = Map<String, Value>;

fn req_str<'a>(a: &'a Args, k: &str) -> Result<&'a str, String> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing string argument `{k}`"))
}

fn exec(b: &mut dyn Backend, cmd: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": cmd, "params": params}))
}

fn need_ui(b: &dyn Backend, tool: &str) -> Result<(), String> {
    if b.has_ui() {
        Ok(())
    } else {
        Err(format!(
            "`{tool}` needs the desktop app: run `vectorcraft --control 7979` and start the server with `vectorcraft-cli mcp --connect 127.0.0.1:7979`"
        ))
    }
}

fn created_id(r: &Value) -> Option<Value> {
    r.get("id").filter(|v| v.is_u64()).map(|id| json!([id]))
}

fn filter_commands(all: Value, a: &Args) -> Value {
    let needle = a.get("filter").and_then(Value::as_str).map(str::to_lowercase).filter(|s| !s.is_empty());
    let enabled_only = a.get("enabled_only").and_then(Value::as_bool).unwrap_or(false);
    let Value::Array(list) = all else { return all };
    let hit = |c: &Value| {
        if enabled_only && c.get("enabled").and_then(Value::as_bool) == Some(false) {
            return false;
        }
        let Some(n) = &needle else { return true };
        let field = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_lowercase();
        let menu = c
            .get("menu")
            .and_then(Value::as_array)
            .map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .to_lowercase();
        field("id").contains(n) || field("label").contains(n) || menu.contains(n)
    };
    Value::Array(list.into_iter().filter(hit).collect())
}

fn dispatch(b: &mut dyn Backend, name: &str, a: &Args) -> Result<ToolResult, String> {
    let j = |v: Value| Ok(ToolResult::json(&v));
    match name {
        "command_list" => {
            let all = b.call("engine.commands", json!({}))?;
            j(filter_commands(all, a))
        }
        "command_batch" => command_batch(b, a),
        "render_preview" => render_preview(b, a),
        "ui_screenshot" => screenshot(b, &Map::from_iter([("window".into(), json!(true))])),
        "command_run" | "invoke_menu" => {
            let cmd = req_str(a, if name == "command_run" { "id" } else { "command" })?;
            let params = match a.get("params") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => return Err("`params` must be an object".into()),
            };
            let method = if name == "invoke_menu" && b.has_ui() { "ui.menu.invoke" } else { "engine.execute" };
            j(b.call(method, json!({"command": cmd, "params": params}))?)
        }
        "doc_inspect" => {
            // Only the slicing options reach the command, which validates them.
            let slice: Map<String, Value> =
                a.iter().filter(|(k, _)| matches!(k.as_str(), "depth" | "childLimit")).map(|(k, v)| (k.clone(), v.clone())).collect();
            j(b.call("document.inspect", Value::Object(slice))?)
        }
        "ui_inspect" => {
            need_ui(b, name)?;
            j(b.call("ui.inspect", json!({}))?)
        }
        "select_tool" => j(b.call("ui.tool.select", json!({"tool": req_str(a, "tool")?}))?),
        "pointer_gesture" => {
            let events = a.get("events").and_then(Value::as_array).filter(|e| !e.is_empty()).ok_or("`events` must be a non-empty array")?;
            for (i, e) in events.iter().enumerate() {
                let kind = e.get("kind").and_then(Value::as_str).unwrap_or("");
                if crate::headless::pointer_kind(kind).is_none() {
                    return Err(format!("event {i}: unknown kind `{kind}` (down, drag, up, move, doubleclick)"));
                }
                if !e.get("x").is_some_and(Value::is_number) || !e.get("y").is_some_and(Value::is_number) {
                    return Err(format!("event {i}: numeric `x` and `y` required"));
                }
            }
            if let Some(t) = a.get("tool").and_then(Value::as_str) {
                b.call("ui.tool.select", json!({"tool": t}))?;
            }
            let events: Vec<Value> = events
                .iter()
                .map(|e| {
                    let mut e = e.clone();
                    e["space"] = json!("doc");
                    e
                })
                .collect();
            let mut p = json!({"events": events});
            if let Some(m) = a.get("mods") {
                p["mods"] = m.clone();
            }
            j(b.call("ui.pointer", p)?)
        }
        "draw_path" => j(draw_path(b, a)?),
        "draw_shape" => j(draw_shape(b, a)?),
        "set_paint" => {
            if !["fill", "stroke", "strokeWidth"].iter().any(|k| a.contains_key(*k)) {
                return Err("give at least one of `fill`, `stroke`, `strokeWidth`".into());
            }
            let ids = a.get("ids").cloned();
            j(json!({"applied": apply_paint(b, a, ids)?}))
        }
        "press_key" => {
            let mut p = json!({"key": req_str(a, "key")?});
            if let Some(Value::Object(m)) = a.get("mods") {
                for (k, v) in m {
                    p[k.as_str()] = v.clone();
                }
            }
            j(b.call("ui.key", p)?)
        }
        "type_text" => {
            need_ui(b, name)?;
            j(b.call("ui.text", json!({"text": req_str(a, "text")?}))?)
        }
        "open_panel" => {
            need_ui(b, name)?;
            j(b.call("ui.set", json!({"panel": req_str(a, "panel")?}))?)
        }
        "screenshot" => screenshot(b, a),
        "open_file" => j(b.call("app.open", json!({"path": req_str(a, "path")?}))?),
        "save_file" => j(b.call("app.save", json!({"path": a.get("path").and_then(Value::as_str)}))?),
        "export" => {
            // One params object for every backend; the named arguments win over `options` (a named
            // artboard or range replaces every artboard choice in `options`).
            let mut params = a.get("options").and_then(Value::as_object).cloned().unwrap_or_default();
            if ["artboard", "range"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_null())) {
                for k in ARTBOARD_PARAMS {
                    params.remove(k);
                }
            }
            for k in ["path", "format", "scale", "artboard", "range", "outlineText"] {
                if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
                    params.insert(k.into(), v.clone());
                }
            }
            let cmd = if a.get("selection").and_then(Value::as_bool) == Some(true) { "document.exportSelection" } else { "document.export" };
            j(exec(b, cmd, Value::Object(params))?)
        }
        "add_text" => j(add_text(b, a)?),
        "apply_effect" => {
            let Some(effect) = a.get("effect").and_then(Value::as_str) else {
                let all = exec(b, "effect.list", json!({}))?;
                return j(json!({"catalog": all.get("catalog")}));
            };
            let mut p = json!({"effect": effect, "params": a.get("params").cloned().unwrap_or(json!({}))});
            if let Some(ids) = a.get("ids") {
                p["ids"] = ids.clone();
            }
            j(exec(b, "effect.apply", p)?)
        }
        "pathfinder" => {
            select_ids(b, a)?;
            j(exec(b, &format!("object.pathfinder.{}", req_str(a, "operation")?), json!({}))?)
        }
        "transform" => j(transform(b, a)?),
        "create_graph" => {
            let mut p = Value::Object(a.clone());
            if p.get("type").is_none() {
                p["type"] = json!("column");
            }
            j(exec(b, "graph.create", p)?)
        }
        "text_wrap" => {
            select_ids(b, a)?;
            if a.get("release").and_then(Value::as_bool) == Some(true) {
                return j(exec(b, "object.textWrap.release", json!({}))?);
            }
            let mut p = json!({});
            for k in ["offset", "invert"] {
                if let Some(v) = a.get(k) {
                    p[k] = v.clone();
                }
            }
            j(exec(b, "object.textWrap.make", p)?)
        }
        "undo" => j(exec(b, "edit.undo", json!({}))?),
        "redo" => j(exec(b, "edit.redo", json!({}))?),
        other => Err(format!("unknown tool `{other}` (see tools/list)")),
    }
}

/// Run one tool. Failures (unknown tool, bad arguments, command errors) come back as an
/// `isError` result so the model can read and correct them.
pub fn call_tool(b: &mut dyn Backend, name: &str, args: &Value) -> ToolResult {
    let empty = Map::new();
    let a = match args {
        Value::Object(o) => o,
        Value::Null => &empty,
        _ => return ToolResult::error("tool arguments must be a JSON object"),
    };
    match vectorcraft_engine::guard::catch_panic(|| dispatch(b, name, a)) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => ToolResult::error(format!("{name}: {e}")),
        Err(e) => ToolResult::error(format!("internal error in `{name}`: {e} (please report this bug)")),
    }
}

fn command_batch(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let steps = a.get("steps").and_then(Value::as_array).ok_or("`steps` must be an array")?;
    let stop = match a.get("stop_on_error") {
        None => true,
        Some(v) => v.as_bool().ok_or("`stop_on_error` must be a boolean")?,
    };
    let (mut completed, mut failed, mut results) = (0, 0, Vec::new());
    for step in steps {
        let result = vectorcraft_engine::guard::catch_panic(|| {
            let args = step.as_object().ok_or("each step must be an object")?;
            if args.keys().any(|k| k != "id" && k != "params") {
                return Err("unknown step argument (expected id, params)".into());
            }
            let id = req_str(args, "id")?;
            let params = match args.get("params") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => return Err("`params` must be an object".into()),
            };
            b.call("engine.execute", json!({"command":id,"params":params}))
        })
        .unwrap_or_else(|e| Err(format!("internal error: {e}")));
        match result {
            Ok(v) => {
                completed += 1;
                results.push(json!({"ok":true,"result":v}));
            }
            Err(e) => {
                failed += 1;
                results.push(json!({"ok":false,"error":e}));
                if stop {
                    break;
                }
            }
        }
    }
    let mut r = ToolResult::json(&json!({"completed":completed,"failed":failed,"results":results}));
    r.is_error = failed > 0;
    Ok(r)
}
