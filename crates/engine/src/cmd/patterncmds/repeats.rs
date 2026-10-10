//! Live repeat creation, arrangement, release and expansion.

use super::super::*;
use super::{Arc, Document, Node, NodeId, NodeKind, Rect, RepeatKind, RepeatSpec, TileType, Value, ids_json, json, pattern, roots};

fn selected_repeats(s: &Session) -> Vec<NodeId> {
    let Some(st) = s.active() else { return vec![] };
    let mut out = vec![];
    for id in &st.selection.objects {
        for a in st.doc.ancestry(*id).unwrap_or_default().into_iter().rev() {
            if st.doc.node(a).is_some_and(pattern::is_repeat) {
                if !out.contains(&a) {
                    out.push(a);
                }
                break;
            }
        }
    }
    out
}

pub(super) fn has_repeat(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_repeats(s).is_empty() { Err("select a repeat".into()) } else { Ok(()) }
}

// ---------- Repeat ----------

/// Make a repeat from the selection, or switch the selected repeat's kind.
fn make_repeat(s: &mut Session, p: &Value, label: &str, make: impl Fn(Vec<Arc<Node>>) -> RepeatSpec) -> Result<Value> {
    let existing = selected_repeats(s);
    if ids_param(p, "ids").is_none()
        && let [r] = existing.as_slice()
    {
        let r = *r;
        s.edit(label, |d, _| {
            let n = d.node_mut(r).ok_or(EngineError::NoNode(r))?;
            if let NodeKind::Repeat(spec) = &mut n.kind {
                spec.kind = make(spec.source.clone()).kind;
            }
            Ok(())
        })?;
        return Ok(json!({ "id": r.0 }));
    }
    let roots = roots(s, p)?;
    let Some(&top) = roots.last() else { return Err(EngineError::Other("select the art to repeat".into())) };
    let id = s.edit(label, |d, sel| {
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let nodes: Vec<Arc<Node>> = roots.iter().filter_map(|id| d.node(*id).cloned()).map(Arc::new).collect();
        let id = d.alloc_id();
        d.insert(par, idx + 1, Node::new(id, NodeKind::Repeat(make(nodes))))?;
        for r in &roots {
            d.remove(*r)?;
        }
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

pub(super) fn repeat_radial(s: &mut Session, p: &Value) -> Result<Value> {
    let n = f64_or(p, "instances", 8.0);
    if !(1.0..=1000.0).contains(&n) {
        return Err(bad("object.repeat.radial", "instances must be 1..1000"));
    }
    let radius = p.get("radius").and_then(Value::as_f64).filter(|r| r.is_finite() && *r >= 0.0);
    make_repeat(s, p, "Radial Repeat", |src| RepeatSpec::radial(src, n as u32, radius))
}

pub(super) fn repeat_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let (hs, vs) = (p.get("hSpacing").and_then(Value::as_f64), p.get("vSpacing").and_then(Value::as_f64));
    let rows = p.get("rows").and_then(Value::as_u64).map(|v| v.clamp(1, 500) as u32);
    let cols = p.get("cols").and_then(Value::as_u64).map(|v| v.clamp(1, 500) as u32);
    make_repeat(s, p, "Grid Repeat", |src| {
        let b = vectorcraft_doc::live::nodes_bounds(&src).unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut r = RepeatSpec::grid(src, hs.unwrap_or(b.width() / 4.0), vs.unwrap_or(b.height() / 4.0));
        if let RepeatKind::Grid { rows: rr, cols: cc, .. } = &mut r.kind {
            *rr = rows.unwrap_or(*rr);
            *cc = cols.unwrap_or(*cc);
        }
        r
    })
}

pub(super) fn repeat_mirror(s: &mut Session, p: &Value) -> Result<Value> {
    let angle = f64_or(p, "angle", 90.0);
    let offset = f64_or(p, "offset", 10.0);
    make_repeat(s, p, "Mirror Repeat", |src| RepeatSpec::mirror(src, angle, offset))
}

pub(super) fn repeat_release(s: &mut Session, _: &Value) -> Result<Value> {
    let reps = selected_repeats(s);
    let ids = s.edit("Release Repeat", |d, sel| {
        let mut out = vec![];
        for r in &reps {
            let Some(n) = d.node(*r).cloned() else { continue };
            let (par, idx, _) = d.position(*r).ok_or(EngineError::NoNode(*r))?;
            d.remove(*r)?;
            for (k, c) in n.children().into_iter().flatten().enumerate() {
                out.push(c.id);
                d.insert(par, idx + k, (**c).clone())?;
            }
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

fn fix_ids(d: &mut Document, n: &mut Node) {
    if n.id == NodeId(0) {
        n.id = d.alloc_id();
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            fix_ids(d, Arc::make_mut(c));
        }
    }
}

pub(super) fn repeat_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let reps = selected_repeats(s);
    let ids = s.edit("Expand Repeat", |d, sel| {
        let mut out = vec![];
        for r in &reps {
            let Some(n) = d.node(*r).cloned() else { continue };
            let mut g = vectorcraft_doc::live::expanded_group(&n, None);

            // The group keeps the repeat's id; every generated instance node gets a fresh one.
            if let Some(ch) = g.children_mut() {
                for c in ch.iter_mut() {
                    fix_ids(d, Arc::make_mut(c));
                }
            }
            let (par, idx, _) = d.position(*r).ok_or(EngineError::NoNode(*r))?;
            d.remove(*r)?;
            d.insert(par, idx, g)?;
            out.push(*r);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

pub(super) fn repeat_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.repeat.options";
    let reps = selected_repeats(s);
    let num = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
    let flag = |k: &str| p.get(k).and_then(Value::as_bool);
    if num("instances").is_some_and(|n| !(1.0..=1000.0).contains(&n)) {
        return Err(bad(C, "instances must be 1..1000"));
    }
    let grid_type = match str_param(p, "gridType") {
        Some(t) => Some(TileType::parse(t, num("brickOffset")).ok_or_else(|| bad(C, format!("unknown gridType `{t}`")))?),
        None => None,
    };
    let center = point_param(p, "center");
    s.edit("Repeat Options", |d, _| {
        for r in &reps {
            let Some(NodeKind::Repeat(spec)) = d.node_mut(*r).map(|n| &mut n.kind) else { continue };
            match &mut spec.kind {
                RepeatKind::Radial { instances, radius, center: c, reverse_overlap, start_angle, end_angle } => {
                    if let Some(v) = num("instances") {
                        *instances = v as u32;
                    }
                    if let Some(v) = num("radius") {
                        *radius = v.max(0.0);
                    }
                    if let Some(v) = flag("reverseOverlap") {
                        *reverse_overlap = v;
                    }
                    if let Some(v) = num("startAngle") {
                        *start_angle = v;
                    }
                    if let Some(v) = num("endAngle") {
                        *end_angle = v;
                    }
                    if let Some(v) = center {
                        *c = v;
                    }
                }
                RepeatKind::Grid { h_spacing, v_spacing, rows, cols, grid_type: gt, flip_rows, flip_cols } => {
                    if let Some(v) = num("hSpacing") {
                        *h_spacing = v;
                    }
                    if let Some(v) = num("vSpacing") {
                        *v_spacing = v;
                    }
                    if let Some(v) = num("rows") {
                        *rows = (v as u32).clamp(1, 500);
                    }
                    if let Some(v) = num("cols") {
                        *cols = (v as u32).clamp(1, 500);
                    }
                    if let Some(t) = grid_type {
                        *gt = t;
                    }
                    if let Some(v) = flag("flipRows") {
                        *flip_rows = v;
                    }
                    if let Some(v) = flag("flipCols") {
                        *flip_cols = v;
                    }
                }
                RepeatKind::Mirror { angle, center: c } => {
                    if let Some(v) = num("angle") {
                        *angle = v;
                    }
                    if let Some(v) = center {
                        *c = v;
                    }
                }
            }
        }
        Ok(())
    })?;
    ok()
}
