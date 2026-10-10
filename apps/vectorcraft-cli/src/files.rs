//! CLI file conversion and document inspection.

use super::{Backend, Headless, Value, json};

pub(super) fn convert(args: &[String]) -> Result<(), String> {
    let mut files = vec![];
    let (mut scale, mut artboard, mut range, mut outline_text, mut replace_lossy) = (1.0f64, None::<u64>, None::<String>, false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--scale" | "-s" => scale = it.next().and_then(|v| v.parse().ok()).ok_or("--scale needs a number")?,
            "--artboard" | "-a" => artboard = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--artboard needs an index")?),
            "--range" | "-r" => range = Some(it.next().cloned().ok_or("--range needs artboards such as 1-3,5")?),
            "--outline-text" => outline_text = true,
            "--replace-lossy" => replace_lossy = true,
            f => files.push(f.to_string()),
        }
    }
    let [input, output] = <[String; 2]>::try_from(files).map_err(|_| "convert needs IN and OUT")?;
    let mut h = Headless::new();
    h.call("app.open", json!({"path": input})).map_err(|e| format!("open {input}: {e}"))?;
    // A conversion keeps hidden layers and objects, written hidden, where the format can (SVG,
    // PSD).
    let params = json!({"path": output, "scale": scale, "artboard": artboard, "range": range, "outlineText": outline_text, "hiddenLayers": true, "acknowledgeLoss": replace_lossy});
    let r = h.call("engine.execute", json!({"command": "document.export", "params": params})).map_err(|e| format!("export {output}: {e}"))?;
    outln!("{r}");
    Ok(())
}

pub(super) fn info(args: &[String]) -> Result<(), String> {
    let file = args.first().ok_or("info needs a FILE")?;
    let mut h = Headless::new();
    let opened = h.call("app.open", json!({"path": file})).map_err(|e| format!("open {file}: {e}"))?;
    let base = h.call("engine.execute", json!({"command": "file.info", "params": {}}))?;
    let doc = h.session.doc().map_err(|e| e.to_string())?.doc.clone();
    let mut kinds: std::collections::BTreeMap<&'static str, usize> = Default::default();
    doc.walk(|n| *kinds.entry(n.kind_label()).or_default() += 1);
    let fonts = h.call("engine.execute", json!({"command": "text.fonts", "params": {}})).unwrap_or(Value::Null);
    let artboards: Vec<Value> =
        doc.artboards.iter().map(|a| json!({"name": a.name, "rect": [a.rect.x0, a.rect.y0, a.rect.width(), a.rect.height()]})).collect();
    // What didn't come in as it was (an EPS read from its preview says why).
    let v = json!({"file": file, "warnings": opened["warnings"], "info": base, "artboards": artboards, "kinds": kinds, "fonts": fonts});
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}
