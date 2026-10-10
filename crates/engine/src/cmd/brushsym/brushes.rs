//! Brush definitions, libraries, stroke application and expansion.

use super::super::*;
use super::{
    AppearanceItem, Brush, Color, Document, EngineError, NodeId, NodeKind, Paint, Value, brush, ids_json, json, name_param, roots, selection_art,
    set_unknown,
};

// ---------- brushes ----------

/// Brushable nodes (paths and compound paths) under `ids`.
fn brush_targets(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for id in ids {
        let Some(n) = d.node(*id) else { continue };
        n.walk(&mut |c| {
            let own = matches!(c.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. });
            let in_compound = d.parent_of(c.id).and_then(|p| d.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
            if own && !in_compound && !out.contains(&c.id) {
                out.push(c.id);
            }
        });
    }
    out
}

pub(super) fn brush_list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let list: Vec<Value> = brush::library(d).iter().map(|b| json!({"name": b.name, "type": b.kind.type_id()})).collect();
    Ok(json!({ "brushes": list, "current": brush::current(d) }))
}

pub(super) fn brush_get(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "brush.get")?;
    let b = brush::find(&s.doc()?.doc, name).ok_or_else(|| EngineError::Other(format!("no brush named `{name}`")))?;
    Ok(serde_json::to_value(b).unwrap_or_default())
}

pub(super) fn brush_apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.apply";
    let name = name_param(p, C)?.to_string();
    if brush::find(&s.doc()?.doc, &name).is_none() {
        return Err(bad(C, format!("no brush named `{name}`")));
    }
    let ids = brush_targets(&s.doc()?.doc, &roots(s, p)?);
    let stroke = if s.paint.stroke.is_none() { Paint::solid(Color::BLACK) } else { s.paint.stroke.clone() };
    s.edit("Apply Brush", |d, _| {
        d.unknown.insert(brush::CURRENT_KEY.into(), json!(name));
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            if n.appearance.stroke().is_none_or(|st| st.paint.is_none()) {
                n.appearance.set_stroke(stroke.clone());
            }
            if let Some(st) = n.appearance.stroke_mut() {
                if st.width <= 0.0 {
                    st.width = 1.0;
                }
                st.brush = Some(name.clone());
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

pub(super) fn brush_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = brush_targets(&s.doc()?.doc, &roots(s, p)?);
    s.edit("Remove Brush Stroke", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                for it in &mut n.appearance.items {
                    if let AppearanceItem::Stroke(st) = it {
                        st.brush = None;
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

pub(super) fn brush_set_current(s: &mut Session, p: &Value) -> Result<Value> {
    match str_param(p, "name") {
        Some(n) => {
            if brush::find(&s.doc()?.doc, n).is_none() {
                return Err(bad("brush.setCurrent", format!("no brush named `{n}`")));
            }
            set_unknown(s, brush::CURRENT_KEY, Some(json!(n)))?;
        }
        None => set_unknown(s, brush::CURRENT_KEY, None)?,
    }
    ok()
}

fn merge(base: &mut Value, patch: &Value) {
    if let (Some(b), Some(pt)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in pt {
            b.insert(k.clone(), v.clone());
        }
    }
}

fn save_library(s: &mut Session, label: &str, lib: Vec<Brush>, rename: Option<(String, Option<String>)>) -> Result<()> {
    s.edit(label, |d, _| {
        brush::store(d, &lib);
        if let Some((from, to)) = rename {
            let ids: Vec<NodeId> = {
                let mut v = vec![];
                d.walk(|n| {
                    if n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if st.brush.as_deref() == Some(from.as_str()))) {
                        v.push(n.id);
                    }
                });
                v
            };
            for id in ids {
                if let Some(n) = d.node_mut(id) {
                    for it in &mut n.appearance.items {
                        if let AppearanceItem::Stroke(st) = it
                            && st.brush.as_deref() == Some(from.as_str())
                        {
                            st.brush = to.clone();
                        }
                    }
                }
            }
            if brush::current(d).as_deref() == Some(from.as_str()) {
                match to {
                    Some(t) => d.unknown.insert(brush::CURRENT_KEY.into(), json!(t)),
                    None => d.unknown.remove(brush::CURRENT_KEY),
                };
            }
        }
        Ok(())
    })
}

pub(super) fn brush_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.new";
    let ty = str_param(p, "type").ok_or_else(|| bad(C, "missing `type`"))?;
    let art_key = match ty {
        "calligraphic" | "bristle" => None,
        "scatter" | "art" => Some("art"),
        "pattern" => Some("side"),
        o => return Err(bad(C, format!("unknown brush type `{o}`"))),
    };
    let mut lib = brush::library(&s.doc()?.doc);
    let base_name = str_param(p, "name").map(str::to_string).unwrap_or_else(|| format!("New {} Brush", ty[..1].to_uppercase() + &ty[1..]));
    let name = brush::unique_name(&lib, &base_name);
    let mut v = json!({"name": name, "type": ty});
    if let Some(params) = p.get("params") {
        merge(&mut v, params);
        v["name"] = json!(name);
        v["type"] = json!(ty);
    }
    if let Some(k) = art_key
        && v.get(k).is_none()
    {
        let art = selection_art(s, p)?.ok_or_else(|| bad(C, format!("{ty} brushes need selected art (or params.{k})")))?;
        if art.geometric_bounds().is_none_or(|b| b.width() <= 1e-6) {
            return Err(bad(C, "the selected art has no width"));
        }
        v[k] = serde_json::to_value(&art).unwrap_or_default();
    }
    let b: Brush = serde_json::from_value(v).map_err(|e| bad(C, format!("invalid brush: {e}")))?;
    lib.push(b);
    save_library(s, "New Brush", lib, None)?;
    Ok(json!({ "name": name }))
}

pub(super) fn brush_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "brush.delete")?.to_string();
    let mut lib = brush::library(&s.doc()?.doc);
    let n = lib.len();
    lib.retain(|b| b.name != name);
    if lib.len() == n {
        return Err(bad("brush.delete", format!("no brush named `{name}`")));
    }
    save_library(s, "Delete Brush", lib, Some((name, None)))?;
    ok()
}

pub(super) fn brush_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "brush.duplicate")?;
    let mut lib = brush::library(&s.doc()?.doc);
    let mut b = lib.iter().find(|b| b.name == name).cloned().ok_or_else(|| bad("brush.duplicate", format!("no brush named `{name}`")))?;
    b.name = brush::unique_name(&lib, str_param(p, "newName").unwrap_or(&format!("{name} copy")));
    let out = b.name.clone();
    lib.push(b);
    save_library(s, "Duplicate Brush", lib, None)?;
    Ok(json!({ "name": out }))
}

pub(super) fn brush_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.options";
    let name = name_param(p, C)?.to_string();
    let mut lib = brush::library(&s.doc()?.doc);
    let i = lib.iter().position(|b| b.name == name).ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
    let mut v = serde_json::to_value(&lib[i]).unwrap_or_default();
    if let Some(params) = p.get("params") {
        merge(&mut v, params);
    }
    let new_name = str_param(p, "newName").filter(|n| !n.trim().is_empty() && *n != name).map(str::to_string);
    if let Some(nn) = &new_name {
        if lib.iter().any(|b| &b.name == nn) {
            return Err(bad(C, format!("a brush named `{nn}` exists")));
        }
        v["name"] = json!(nn);
    } else {
        v["name"] = json!(name);
    }
    v["type"] = json!(lib[i].kind.type_id());
    lib[i] = serde_json::from_value(v).map_err(|e| bad(C, format!("invalid brush: {e}")))?;
    let out = lib[i].name.clone();
    let rename = new_name.map(|nn| (name, Some(nn)));
    save_library(s, "Brush Options", lib, rename)?;
    Ok(json!({ "name": out }))
}

pub(super) fn expand_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = brush_targets(&s.doc()?.doc, &roots(s, p)?);
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| s.doc().ok().and_then(|st| st.doc.node(*id)).is_some_and(brush::has_brush)).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("Expand: no brush strokes selected".into()));
    }
    let out = s.edit("Expand", |d, sel| {
        let mut out = vec![];
        for id in &ids {
            let Some(n) = d.node(*id).cloned() else { continue };
            let Some(g) = brush::expand(d, &n) else { continue };
            let g = d.reid(&g);
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            let gid = g.id;
            d.remove(*id)?;
            d.insert(par, idx, g)?;
            out.push(gid);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&out) }))
}

pub(super) fn brush_freehand(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "brush").map(str::to_string).filter(|n| s.doc().is_ok_and(|st| brush::find(&st.doc, n).is_some()));
    let spec = find_command("path.freehand").ok_or_else(|| EngineError::UnknownCommand("path.freehand".into()))?;
    let r = (spec.run)(s, p)?;
    let (Some(name), Some(id)) = (name, id_param(&r, "id")) else { return Ok(r) };
    s.edit("Paintbrush", |d, _| {
        if let Some(st) = d.node_mut(id).and_then(|n| n.appearance.stroke_mut()) {
            st.brush = Some(name.clone());
        }
        Ok(())
    })?;
    Ok(r)
}
