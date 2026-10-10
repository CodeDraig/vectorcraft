//! Drawing, paint, text and transform tool arguments and command translation.

use super::{Args, Backend, Map, Value, created_id, exec, json, req_str};

/// Paint argument → `paint.setFill` / `paint.setStroke` params.
fn paint_params(v: &Value) -> Result<Value, String> {
    Ok(match v {
        Value::Null => json!({"none": true}),
        Value::String(s) if s.eq_ignore_ascii_case("none") => json!({"none": true}),
        Value::String(_) | Value::Array(_) => json!({"color": v}),
        Value::Object(o) if ["color", "none", "swatch", "gradient"].iter().any(|k| o.contains_key(*k)) => v.clone(),
        Value::Object(_) => json!({"color": v}),
        other => return Err(format!("unsupported paint value {other}")),
    })
}

pub(super) fn apply_paint(b: &mut dyn Backend, a: &Args, ids: Option<Value>) -> Result<Value, String> {
    let with_ids = |mut p: Value| {
        if let Some(ids) = &ids {
            p["ids"] = ids.clone();
        }
        p
    };
    let mut applied = Map::new();
    if let Some(f) = a.get("fill") {
        exec(b, "paint.setFill", with_ids(paint_params(f)?))?;
        applied.insert("fill".into(), f.clone());
    }
    if let Some(s) = a.get("stroke") {
        exec(b, "paint.setStroke", with_ids(paint_params(s)?))?;
        applied.insert("stroke".into(), s.clone());
    }
    if let Some(w) = a.get("strokeWidth") {
        let w = w.as_f64().ok_or("`strokeWidth` must be a number")?;
        exec(b, "stroke.set", with_ids(json!({"weight": w})))?;
        applied.insert("strokeWidth".into(), json!(w));
    }
    Ok(Value::Object(applied))
}

pub(super) fn draw_path(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let mut p = json!({});
    if let Some(d) = a.get("d").and_then(Value::as_str) {
        p["d"] = json!(d);
    } else {
        let pts = a.get("points").and_then(Value::as_array).ok_or("give `points` or `d`")?;
        let mut anchors = vec![];
        for (i, pt) in pts.iter().enumerate() {
            let anchor = match pt {
                Value::Array(xy) if xy.len() >= 2 && xy[0].is_number() && xy[1].is_number() => json!({"x": xy[0], "y": xy[1]}),
                Value::Object(o) if o.get("x").is_some_and(Value::is_number) && o.get("y").is_some_and(Value::is_number) => pt.clone(),
                _ => return Err(format!("point {i} must be [x,y] or {{x,y,…}}")),
            };
            anchors.push(anchor);
        }
        if anchors.is_empty() {
            return Err("`points` is empty".into());
        }
        p["anchors"] = Value::Array(anchors);
        p["closed"] = json!(a.get("closed").and_then(Value::as_bool).unwrap_or(false));
    }
    let r = exec(b, "path.create", p)?;
    let paint = apply_paint(b, a, created_id(&r))?;
    Ok(json!({"id": r.get("id"), "paint": paint}))
}

pub(super) fn draw_shape(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let shape = req_str(a, "shape")?;
    let cmd = match shape {
        "rectangle" | "rect" => "shape.rectangle",
        "ellipse" | "circle" => "shape.ellipse",
        "polygon" => "shape.polygon",
        "star" => "shape.star",
        "line" => "shape.line",
        other => return Err(format!("unknown shape `{other}` (rectangle, ellipse, polygon, star, line)")),
    };
    let mut p = a.clone();
    for k in ["shape", "fill", "stroke", "strokeWidth"] {
        p.remove(k);
    }
    let r = exec(b, cmd, Value::Object(p))?;
    let paint = apply_paint(b, a, created_id(&r))?;
    Ok(json!({"id": r.get("id"), "shape": shape, "paint": paint}))
}

/// Select `ids` when given (tools that act on the selection).
pub(super) fn select_ids(b: &mut dyn Backend, a: &Args) -> Result<(), String> {
    if let Some(ids) = a.get("ids") {
        exec(b, "select.set", json!({"ids": ids}))?;
    }
    Ok(())
}

pub(super) fn add_text(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let text = a.get("text").and_then(Value::as_str).ok_or("missing string argument `text`")?;
    let mut style = json!({});
    for k in ["size", "font"] {
        if let Some(v) = a.get(k) {
            style[k] = v.clone();
        }
    }
    let r = if let Some(path) = a.get("path") {
        let mut p = json!({"path": path, "mode": a.get("mode").cloned().unwrap_or(json!("onPath")), "text": text});
        for (k, v) in style.as_object().into_iter().flatten() {
            p[k.as_str()] = v.clone();
        }
        exec(b, "text.createInPath", p)?
    } else {
        let x = a.get("x").and_then(Value::as_f64).ok_or("give `x` and `y` (or a `path`)")?;
        let y = a.get("y").and_then(Value::as_f64).ok_or("give `x` and `y` (or a `path`)")?;
        let mut p = json!({"x": x, "y": y, "text": text});
        for (k, v) in style.as_object().into_iter().flatten() {
            p[k.as_str()] = v.clone();
        }
        if let Some(c) = a.get("color") {
            p["color"] = c.clone();
        }
        if let (Some(w), Some(h)) = (a.get("width"), a.get("height")) {
            p["area"] = json!({"width": w, "height": h});
        }
        exec(b, "text.create", p)?
    };
    if let Some(e) = a.get("pathEffect").and_then(Value::as_str) {
        exec(b, "type.pathOptions", json!({"effect": e}))?;
    }
    Ok(json!({"id": r.get("id")}))
}

pub(super) fn transform(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    select_ids(b, a)?;
    // With `copy`, the first operation duplicates and the rest transform the copy (the new selection).
    let copy = std::cell::Cell::new(a.get("copy").and_then(Value::as_bool).unwrap_or(false));
    let with = |mut p: Value| {
        if let Some(o) = a.get("origin") {
            p["origin"] = o.clone();
        }
        p["copy"] = json!(copy.replace(false));
        p
    };
    let mut done = vec![];
    if a.contains_key("dx") || a.contains_key("dy") {
        let num = |k: &str| a.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        exec(b, "object.move", json!({"dx": num("dx"), "dy": num("dy"), "copy": copy.replace(false)}))?;
        done.push("move");
    }
    if let Some(r) = a.get("rotate").and_then(Value::as_f64) {
        exec(b, "object.rotate", with(json!({"angle": r})))?;
        done.push("rotate");
    }
    let s = a.get("scale").and_then(Value::as_f64);
    let (sx, sy) = (a.get("scaleX").and_then(Value::as_f64).or(s), a.get("scaleY").and_then(Value::as_f64).or(s));
    if sx.is_some() || sy.is_some() {
        exec(b, "object.scale", with(json!({"sx": sx.unwrap_or(100.0), "sy": sy.unwrap_or(100.0)})))?;
        done.push("scale");
    }
    if let Some(axis) = a.get("reflect").and_then(Value::as_str) {
        exec(b, "object.reflect", with(json!({"axis": axis})))?;
        done.push("reflect");
    }
    if let Some(sh) = a.get("shear").and_then(Value::as_f64) {
        exec(b, "object.shear", with(json!({"angle": sh})))?;
        done.push("shear");
    }
    if done.is_empty() {
        return Err("give at least one of dx/dy, rotate, scale/scaleX/scaleY, reflect, shear".into());
    }
    Ok(json!({"applied": done}))
}
