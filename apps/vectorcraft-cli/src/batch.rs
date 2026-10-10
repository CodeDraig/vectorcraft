//! CLI command-script parsing and execution.

use super::{Backend, Headless, Value, Write, json};

enum Step {
    Cmd(String, Value),
    Export(String),
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let mut input: Option<String> = None;
    let mut steps: Vec<Step> = vec![];
    let mut scale = 1.0;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |what: &str| it.next().cloned().ok_or_else(|| format!("{a} needs {what}"));
        match a.as_str() {
            "--in" | "-i" => input = Some(val("a file")?),
            "--cmd" | "-c" => steps.push(Step::Cmd(val("a command id")?, json!({}))),
            "--params" | "-p" => {
                let raw = val("JSON")?;
                let p: Value = serde_json::from_str(&raw).map_err(|e| format!("--params {raw}: {e}"))?;
                if !p.is_object() {
                    return Err(format!("--params must be a JSON object, got {raw}"));
                }
                match steps.last_mut() {
                    Some(Step::Cmd(_, params)) => *params = p,
                    _ => return Err("--params must follow a --cmd".into()),
                }
            }
            "--export" | "-o" => steps.push(Step::Export(val("a file")?)),
            "--scale" | "-s" => {
                let raw = val("a number")?;
                scale = raw.parse::<f64>().map_err(|_| format!("--scale {raw}: not a number"))?;
            }
            other => return Err(format!("unknown run option `{other}`")),
        }
    }

    let mut h = Headless::new();
    let mut out = std::io::stdout().lock();
    let mut emit = |v: Value| writeln!(out, "{v}").map_err(|e| e.to_string());
    if let Some(path) = &input {
        let r = h.call("app.open", json!({"path": path})).map_err(|e| format!("open {path}: {e}"))?;
        emit(json!({"step": "open", "path": path, "result": r}))?;
    } else if !matches!(steps.first(), Some(Step::Cmd(id, _)) if id == "file.new") {
        h.ensure_document();
    }
    for step in steps {
        match step {
            Step::Cmd(id, params) => {
                let r = h.call("engine.execute", json!({"command": id, "params": params})).map_err(|e| format!("{id}: {e}"))?;
                emit(json!({"step": "cmd", "command": id, "result": r}))?;
            }
            Step::Export(path) => {
                let r = h.call("app.export", json!({"path": path, "scale": scale})).map_err(|e| format!("export {path}: {e}"))?;
                emit(json!({"step": "export", "result": r}))?;
            }
        }
    }
    Ok(())
}
