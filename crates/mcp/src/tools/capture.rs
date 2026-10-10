//! Inline screenshots, bounded previews and image content blocks.

use super::{Args, Backend, ToolResult, Value, json, need_ui};

pub(super) fn screenshot(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let path = a.get("path").and_then(Value::as_str);
    if a.get("window").and_then(Value::as_bool) == Some(true) {
        need_ui(b, "screenshot {window:true}")?;
        // The app writes the capture to disk (loopback: same machine), then we read it back.
        let tmp;
        let target = match path {
            Some(p) => p,
            None => {
                tmp = std::env::temp_dir().join(format!("vectorcraft-window-{}.png", std::process::id())).to_string_lossy().to_string();
                &tmp
            }
        };
        let r = b.call("ui.screenshot", json!({"path": target}))?;
        let png = std::fs::read(target).map_err(|e| format!("read {target}: {e}"))?;
        if path.is_none() {
            std::fs::remove_file(target).ok();
        }
        return Ok(image_result(&png, json!({"window": true, "width": r.get("width"), "height": r.get("height"), "path": path})));
    }
    let mut p = json!({});
    for k in ["scale", "artboard"] {
        if let Some(v) = a.get(k) {
            p[k] = v.clone();
        }
    }
    let r = b.call("ui.render", p)?;
    let b64 = r.get("pngBase64").and_then(Value::as_str).ok_or("renderer returned no image")?;
    if let Some(path) = path {
        let png = vectorcraft_format::base64_decode(b64).ok_or("renderer returned bad base64")?;
        std::fs::write(path, png).map_err(|e| format!("write {path}: {e}"))?;
    }
    Ok(ToolResult {
        content: vec![
            json!({"type": "image", "data": b64, "mimeType": "image/png"}),
            json!({"type": "text", "text": json!({"width": r.get("width"), "height": r.get("height"), "path": path}).to_string()}),
        ],
        is_error: false,
    })
}

fn image_result(png: &[u8], info: Value) -> ToolResult {
    ToolResult {
        content: vec![
            json!({"type": "image", "data": vectorcraft_format::base64_encode(png), "mimeType": "image/png"}),
            json!({"type": "text", "text": info.to_string()}),
        ],
        is_error: false,
    }
}

pub(super) fn render_preview(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let max = match a.get("max_side") {
        None => 1024,
        Some(v) => v.as_u64().filter(|v| (1..=4096).contains(v)).ok_or("`max_side` must be an integer in 1..4096")?,
    };
    let doc = b.call("document.inspect", json!({"depth":0}))?;
    let board = doc.get("artboards").and_then(Value::as_array).and_then(|v| v.first()).ok_or("no such artboard")?;
    let width = board.get("width").and_then(Value::as_f64).ok_or("artboard has no width")?;
    let height = board.get("height").and_then(Value::as_f64).ok_or("artboard has no height")?;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("invalid artboard dimensions".into());
    }
    // The headless renderer clamps scale at 0.01. Bound its allocation even for
    // a huge artboard, then downsize to the requested dimensions.
    let scale = (max as f64 / width.max(height)).clamp(0.01, 1.0);
    if width.max(height) * scale > 4096.0 {
        return Err("artboard is too large for a bounded preview".into());
    }
    let r = b.call("ui.render", json!({"scale":scale}))?;
    let encoded = r.get("pngBase64").and_then(Value::as_str).ok_or("renderer returned no image")?;
    let png = vectorcraft_format::base64_decode(encoded).ok_or("renderer returned bad base64")?;
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| format!("read preview: {e}"))?;
    let img = img.thumbnail(max as u32, max as u32);
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("encode preview: {e}"))?;
    Ok(image_result(out.get_ref(), json!({"width":img.width(),"height":img.height()})))
}
