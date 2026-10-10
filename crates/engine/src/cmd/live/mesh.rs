//! Gradient mesh creation and control-point editing.

use super::super::*;
use super::{
    Color, EnvelopeKind, GradientMesh, MeshAppearance, Node, NodeKind, Paint, Point, Value, fix_ids, gradient_sampler, ids_json, is_mesh, json, live,
    outline_of, replace_with, roots_param, rows_cols, selected_of,
};

// ---------- Gradient mesh ----------

pub(super) fn mesh_create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.create";
    let (rows, cols) = rows_cols(p, C, (4, 4))?;
    let app = match str_param(p, "appearance") {
        Some(a) => MeshAppearance::parse(a).ok_or_else(|| bad(C, "appearance must be flat|center|edge"))?,
        None => MeshAppearance::Flat,
    };
    let highlight = f64_or(p, "highlight", 100.0).clamp(0.0, 100.0);
    let at = point_param(p, "at");
    let (rows, cols) = if at.is_some() { (1, 1) } else { (rows, cols) };
    let roots = roots_param(s, p)?;
    let (ids, index) = s.edit("Create Gradient Mesh", |d, sel| {
        let mut out = vec![];
        let mut index = None;
        for id in &roots {
            let Some(n) = d.node(*id).cloned() else { continue };
            let Some(path) = outline_of(&n) else { continue };
            let m = match n.appearance.fill_paint() {
                // The points take the colours the gradient paints there.
                Paint::Gradient(g) => {
                    path.bounds().and_then(|b| GradientMesh::for_path_with(&path, rows, cols, &*gradient_sampler(&g, b))).map(|mut m| {
                        m.highlight(app, highlight);
                        m
                    })
                }
                p => GradientMesh::for_path(&path, rows, cols, p.color().unwrap_or(Color::WHITE), app, highlight),
            };
            let Some(mut m) = m else { continue };
            if let Some(q) = at {
                index = m.add_lines_at(q);
            }
            let mut mn = Node::new(n.id, NodeKind::Mesh(m));
            mn.name = n.name.clone();
            mn.opacity = n.opacity;
            mn.blend = n.blend;
            mn.locked = n.locked;
            mn.visible = n.visible;
            *d.node_mut(*id).ok_or(EngineError::NoNode(*id))? = mn;
            out.push(*id);
        }
        if out.is_empty() {
            return Err(bad(C, "select a path or compound path"));
        }
        sel.set(out.iter().copied());
        Ok((out, index))
    })?;
    let mut r = ids_json(&ids);
    if let Some(i) = index {
        r["index"] = json!(i);
    }
    Ok(r)
}

/// Run `f` on the gradient mesh `p.id` (one undo step `label`); with `envelopes`, a mesh
/// envelope's grid too (its points and handles are written back: editing fills in the handles).
fn with_mesh<T>(s: &mut Session, p: &Value, cmd: &str, label: &str, envelopes: bool, f: impl FnOnce(&mut GradientMesh) -> Result<T>) -> Result<T> {
    let id = id_param(p, "id").ok_or_else(|| bad(cmd, "missing `id`"))?;
    let c = cmd.to_string();
    s.edit(label, move |d, _| match d.node_mut(id).map(|n| &mut n.kind) {
        Some(NodeKind::Mesh(m)) => f(m),
        Some(NodeKind::Envelope { kind: EnvelopeKind::Mesh { rows, cols, points, handles }, .. }) if envelopes => {
            let mut m = live::envelope_grid(*rows, *cols, points, handles).ok_or_else(|| bad(&c, "the envelope's mesh is malformed"))?;
            let r = f(&mut m)?;
            (*rows, *cols) = (m.rows, m.cols);
            *points = m.points.iter().map(|q| q.p).collect();
            *handles = m.points.iter().map(|q| q.handles).collect();
            Ok(r)
        }
        Some(_) => Err(bad(&c, if envelopes { "not a gradient mesh or mesh envelope" } else { "not a gradient mesh" })),
        None => Err(EngineError::NoNode(id)),
    })
}

/// The `handle` parameter (0 right, 1 left, 2 down, 3 up), if any.
pub(super) fn handle_param(p: &Value, cmd: &str) -> Result<Option<usize>> {
    match p.get("handle") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(h) if h <= 3 => Ok(Some(h as usize)),
            _ => Err(bad(cmd, "handle must be 0..3")),
        },
    }
}

pub(super) fn index_param(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    let i = f64_req(p, key, cmd)?;
    if i < 0.0 {
        return Err(bad(cmd, "index must be ≥ 0"));
    }
    Ok(i as usize)
}

pub(super) fn mesh_set_color(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.setPointColor";
    let index = index_param(p, "index", C)?;
    let color = p.get("color").and_then(color_value);
    let opacity = p.get("opacity").and_then(Value::as_f64);
    if color.is_none() && opacity.is_none() {
        return Err(bad(C, "missing `color` or `opacity`"));
    }
    with_mesh(s, p, C, "Mesh Point Color", false, |m| {
        let pt = m.points.get_mut(index).ok_or_else(|| bad(C, "index out of range"))?;
        if let Some(c) = color {
            pt.color = c;
        }
        if let Some(o) = opacity {
            pt.opacity = o.clamp(0.0, 1.0) as f32;
        }
        Ok(())
    })?;
    ok()
}

pub(super) fn mesh_move_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.movePoint";
    let index = index_param(p, "index", C)?;
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let handle = handle_param(p, C)?;
    with_mesh(s, p, C, "Move Mesh Point", true, |m| {
        let pt = m.points.get_mut(index).ok_or_else(|| bad(C, "index out of range"))?;
        match handle {
            Some(h) => pt.handles[h] = q - pt.p,
            None => pt.p = q,
        }
        Ok(())
    })?;
    ok()
}

pub(super) fn mesh_add_line(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.addLine";
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let color = p.get("color").and_then(color_value);
    let index = with_mesh(s, p, C, "Add Mesh Line", true, |m| {
        let i = m.add_lines_at(q).ok_or_else(|| bad(C, "point is outside the mesh"))?;
        if let Some(c) = color {
            m.points[i].color = c;
        }
        Ok(i)
    })?;
    Ok(json!({ "index": index }))
}

pub(super) fn mesh_delete_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.deletePoint";
    let index = index_param(p, "index", C)?;
    with_mesh(s, p, C, "Delete Mesh Point", true, |m| {
        if m.remove_point_lines(index) { Ok(()) } else { Err(bad(C, "only interior mesh lines can be deleted")) }
    })?;
    ok()
}

pub(super) fn mesh_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let meshes = selected_of(s, is_mesh);
    let ids = s.edit("Expand Gradient Mesh", |d, sel| {
        let mut out = vec![];
        for m in &meshes {
            let Some(n) = d.node(*m).cloned() else { continue };
            let mut g = live::expanded_group(&n, None);
            fix_ids(d, &mut g);
            out.extend(replace_with(d, *m, vec![g])?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}
