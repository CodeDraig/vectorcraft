//! Perspective grid state, projection and attached-art edits.

use super::super::*;
use super::{
    Affine, Arc, Document, EngineError, Homography, NodeId, NodeKind, PerspectiveGrid, Plane, Point, Value, collect_points, duplicate_in, json,
    persp, project_node, selected_roots,
};

// ---------- perspective grid ----------

/// The document's perspective grid (the default preset when none was defined).
pub(crate) fn grid_of(d: &Document) -> PerspectiveGrid {
    PerspectiveGrid::effective(d)
}

/// Write the grid into the document (`Document.unknown["perspectiveGrid"]`).
pub(crate) fn store_grid(d: &mut Document, g: &PerspectiveGrid) {
    g.store(d);
}

/// Change view state of the grid without an undo step.
pub(crate) fn silent(s: &mut Session, f: impl FnOnce(&mut PerspectiveGrid)) -> Result<PerspectiveGrid> {
    let st = s.doc_mut()?;
    let mut g = grid_of(&st.doc);
    f(&mut g);
    store_grid(Arc::make_mut(&mut st.doc), &g);
    if let Some(it) = &mut st.interaction {
        store_grid(Arc::make_mut(&mut it.doc), &g);
    }
    st.revision += 1;
    Ok(g)
}

pub(super) fn grid_set(s: &mut Session, p: &Value) -> Result<Value> {
    let old = grid_of(&s.doc()?.doc);
    let g = old.merged(p).map_err(|e| bad("perspective.grid.set", e))?;
    let reproject = bool_or(p, "reproject", false);
    s.edit("Define Perspective Grid", |d, _| {
        if reproject {
            old.adopt_stored_attachments(d);
            for (id, plane, depth) in persp::attached_roots(d) {
                let h = old.homography_at(plane, depth).and_then(|h| h.inverse()).zip(g.homography_at(plane, depth));
                let (hi, h) = h.ok_or_else(|| EngineError::Other("an object in perspective would leave the edited grid".into()))?;
                let m = h.then_after(&hi);
                warp_checked(d, id, &m)?;
            }
        }
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(g.definition_json())
}

pub(super) fn grid_show(s: &mut Session, p: &Value) -> Result<Value> {
    let want = p.get("visible").and_then(Value::as_bool);
    let g = silent(s, |g| g.visible = want.unwrap_or(!g.visible))?;
    Ok(json!({ "visible": g.visible }))
}

pub(super) fn plane_set(s: &mut Session, p: &Value) -> Result<Value> {
    let plane = str_param(p, "plane").and_then(Plane::parse).ok_or_else(|| bad("perspective.plane.set", "plane must be left|right|ground|none"))?;
    silent(s, |g| g.plane = plane)?;
    Ok(json!({ "plane": plane.id() }))
}

fn plane_param(g: &PerspectiveGrid, p: &Value, cmd: &str) -> Result<Plane> {
    let plane = match str_param(p, "plane") {
        Some(v) => Plane::parse(v).ok_or_else(|| bad(cmd, "plane must be left|right|ground"))?,
        None => g.plane,
    };
    if plane == Plane::None {
        return Err(EngineError::Other("no active perspective plane".into()));
    }
    Ok(plane)
}

fn roots(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => Ok(v),
        None => selected_roots(s),
    }
}

/// Map `n` with `f`, failing if any point would leave the plane's visible side.
/// Map `n` with `h` (type and symbols in perspective keep their art and are drawn through it),
/// failing if any point would leave the plane's visible side.
fn warp_checked(d: &mut Document, id: NodeId, h: &Homography) -> Result<()> {
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    let mut pts = vec![];
    collect_points(n, &mut pts);
    if pts.iter().any(|q| h.apply(*q).is_none()) {
        return Err(EngineError::Other("the object would cross the horizon of the perspective plane".into()));
    }
    project_node(n, h);
    Ok(())
}

/// Attach `id` to `plane`: projected by `map`, else by the map that keeps its bounds' corners
/// (on the nearest gridlines with `snap`).
fn attach_in(d: &mut Document, g: &mut PerspectiveGrid, id: NodeId, plane: Plane, snap: bool, map: Option<Homography>) -> Result<()> {
    let b = d.node(id).ok_or(EngineError::NoNode(id))?.geometric_bounds().ok_or_else(|| EngineError::Other("the object has no geometry".into()))?;
    let m =
        map.or_else(|| g.attach_homography(plane, b, snap)).ok_or_else(|| EngineError::Other("the object is beyond the plane's horizon".into()))?;
    warp_checked(d, id, &m)?;
    persp::set_attachment(d.node_mut(id).ok_or(EngineError::NoNode(id))?, plane, g.offset(plane));
    g.attached.insert(id.0.to_string(), plane);
    Ok(())
}

pub(super) fn attach(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = roots(s, p)?;
    let mut g = grid_of(&s.doc()?.doc);
    let plane = plane_param(&g, p, "perspective.attach")?;
    s.edit("Attach to Active Plane", |d, _| {
        for id in &ids {
            attach_in(d, &mut g, *id, plane, false, None)?;
        }
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "plane": plane.id() }))
}

pub(super) fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = roots(s, p)?;
    let mut g = grid_of(&s.doc()?.doc);
    s.edit("Release with Perspective", |d, _| {
        for id in &ids {
            g.attached.remove(&id.0.to_string());
            if let Some(n) = d.node_mut(*id) {
                persp::release(n);
            }
        }
        store_grid(d, &g);
        Ok(())
    })?;
    ok()
}

/// One object's share of a perspective edit: the page map for an object on `plane` at `depth`, and
/// the depth it ends at.
type PerspStep<'a> = &'a dyn Fn(&PerspectiveGrid, Plane, f64) -> Result<(Homography, f64)>;

/// Where an object is in perspective: its plane and depth.
type Attached = (Plane, f64);

fn beyond_horizon() -> EngineError {
    EngineError::Other("the pointer is beyond the plane's horizon".into())
}

/// Edit objects in perspective (one undo step `label`): with `copy` their duplicates (which stay
/// attached), each mapped by `step` for its own plane and depth; objects not in perspective attach
/// to `fallback` first. → the edited objects and where the first one was (plane, depth).
fn persp_edit(
    s: &mut Session,
    label: &str,
    ids: &[NodeId],
    fallback: Option<Plane>,
    copy: bool,
    step: PerspStep,
) -> Result<(Vec<NodeId>, Option<Attached>)> {
    if ids.is_empty() {
        return Err(EngineError::Other("select the objects to transform in perspective".into()));
    }
    let mut g = grid_of(&s.doc()?.doc);
    s.edit(label, |d, sel| {
        g.adopt_stored_attachments(d);
        let targets = if copy { duplicate_in(d, sel, ids, Affine::IDENTITY)? } else { ids.to_vec() };
        let mut first = None;
        for id in &targets {
            let (plane, depth) = match g.attachment_of(d, *id) {
                Some(a) => a,
                None => {
                    let pl = fallback.ok_or_else(|| EngineError::Other("the object isn't on a perspective plane".into()))?;
                    attach_in(d, &mut g, *id, pl, false, None)?;
                    (pl, g.offset(pl))
                }
            };
            first.get_or_insert((plane, depth));
            let (h, depth) = step(&g, plane, depth)?;
            warp_checked(d, *id, &h)?;
            persp::set_attachment(d.node_mut(*id).ok_or(EngineError::NoNode(*id))?, plane, depth);
        }
        store_grid(d, &g);
        Ok((targets, first))
    })
}

/// Remember `again` (`perspective.transform` params) for Object › Transform › Transform Again, once
/// the drag that made it is committed.
fn record_again(s: &mut Session, again: Value) -> Result<()> {
    let st = s.doc_mut()?;
    match &mut st.interaction {
        Some(it) => it.perspective_again = Some(again),
        None => st.last_perspective = Some(again),
    }
    Ok(())
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// A plane-space affine map as `perspective.transform`'s `matrix` param.
fn matrix_json(m: Affine) -> Value {
    json!(m.as_coeffs())
}

pub(super) fn persp_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.move";
    let ids = roots(s, p)?;
    let from = point_param(p, "from").ok_or_else(|| bad(C, "missing from"))?;
    let to = point_param(p, "to").ok_or_else(|| bad(C, "missing to"))?;
    if ![from.x, from.y, to.x, to.y].iter().all(|v| v.is_finite()) {
        return Err(bad(C, "from and to must be finite"));
    }
    let fallback = plane_param(&grid_of(&s.doc()?.doc), p, C).ok();
    let (perp, copy) = (bool_or(p, "perpendicular", false), bool_or(p, "copy", false));
    // Snap to Grid: a correction for the objects on the first one's plane and depth.
    let fix = if perp { None } else { super::super::perspgrid::snap_fix(&s.doc()?.doc, &ids, p, (from, to)) };
    let snapped = move |pl: Plane, depth: f64, dv: vectorcraft_geom::Vec2| {
        dv + fix.filter(|f| f.0 == pl && f.1 == depth).map_or(vectorcraft_geom::Vec2::ZERO, |f| f.2)
    };
    // In-plane: the plane-space offset under the pointer; perpendicular: the depth it reaches.
    let step = |g: &PerspectiveGrid, pl: Plane, depth: f64| -> Result<(Homography, f64)> {
        if perp {
            let to_depth = g.depth_at(pl, depth, from, to).ok_or_else(beyond_horizon)?;
            Ok((g.transform_map(pl, depth, Affine::IDENTITY, to_depth - depth).ok_or_else(beyond_horizon)?, to_depth))
        } else {
            let dv = snapped(pl, depth, g.plane_delta(pl, depth, from, to).ok_or_else(beyond_horizon)?);
            Ok((g.transform_map(pl, depth, Affine::translate(dv), 0.0).ok_or_else(beyond_horizon)?, depth))
        }
    };
    let label = if copy { "Copy in Perspective" } else { "Move in Perspective" };
    let (targets, first) = persp_edit(s, label, &ids, fallback, copy, &step)?;
    // Transform Again repeats the first object's move.
    if let Some((pl, depth)) = first {
        let g = grid_of(&s.doc()?.doc);
        let again = match perp {
            true => g.depth_at(pl, depth, from, to).map(|d| json!({"matrix": matrix_json(Affine::IDENTITY), "depth": d - depth, "copy": copy})),
            false => {
                g.plane_delta(pl, depth, from, to).map(|dv| json!({"matrix": matrix_json(Affine::translate(snapped(pl, depth, dv))), "copy": copy}))
            }
        };
        if let Some(a) = again {
            record_again(s, a)?;
        }
    }
    Ok(ids_json(&targets))
}

pub(super) fn persp_transform(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.transform";
    let m = matrix_param(p, "matrix").ok_or_else(|| bad(C, "missing matrix [a,b,c,d,e,f]"))?;
    let dz = f64_or(p, "depth", 0.0);
    if !m.as_coeffs().iter().all(|v| v.is_finite()) || m.determinant().abs() < 1e-12 || !dz.is_finite() {
        return Err(bad(C, "matrix must be finite and invertible, depth finite"));
    }
    let ids = roots(s, p)?;
    let fallback = plane_param(&grid_of(&s.doc()?.doc), p, C).ok();
    let copy = bool_or(p, "copy", false);
    let step = |g: &PerspectiveGrid, pl: Plane, depth: f64| -> Result<(Homography, f64)> {
        let to = depth + dz;
        if to.abs() > persp::MAX_DEPTH {
            return Err(EngineError::Other("the objects would leave the grid".into()));
        }
        Ok((g.transform_map(pl, depth, m, dz).ok_or_else(beyond_horizon)?, to))
    };
    let (targets, _) = persp_edit(s, if copy { "Copy in Perspective" } else { "Transform in Perspective" }, &ids, fallback, copy, &step)?;
    record_again(s, json!({"matrix": matrix_json(m), "depth": dz, "copy": copy}))?;
    Ok(ids_json(&targets))
}

/// Object › Transform › Transform Again after a perspective move or scale: the same plane-space
/// transform on the selection.
pub(crate) fn transform_again(s: &mut Session, again: &Value) -> Result<Value> {
    let mut p = again.clone();
    if let Some(o) = p.as_object_mut() {
        o.remove("ids");
    }
    persp_transform(s, &p)
}

pub(super) fn plane_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.plane.move";
    let old = grid_of(&s.doc()?.doc);
    let plane = plane_param(&old, p, C)?;
    let from = old.offset(plane);
    let to = match (p.get("offset").and_then(Value::as_f64), p.get("by").and_then(Value::as_f64)) {
        (Some(o), _) => o,
        (None, Some(b)) => from + b,
        (None, None) => return Err(bad(C, "give offset or by")),
    };
    let objects = str_param(p, "objects").unwrap_or("none");
    if !matches!(objects, "none" | "move" | "copy") {
        return Err(bad(C, "objects must be none, move or copy"));
    }
    let mut g = old.clone();
    g.set_offset(plane, to);
    g.validate().map_err(|e| bad(C, e))?;
    let label = match objects {
        "copy" => "Move Plane and Copy Objects",
        "move" => "Move Plane and Objects",
        _ => "Move Plane",
    };
    let ids = s.edit(label, |d, sel| {
        old.adopt_stored_attachments(d);
        let mut ids = vec![];
        if objects != "none" {
            // The objects on the plane where it was (not those moved off it along its normal).
            let on: Vec<NodeId> = persp::attached_roots(d)
                .into_iter()
                .filter(|(id, pl, depth)| *pl == plane && (depth - from).abs() < 1e-6 && d.is_editable(*id))
                .map(|(id, ..)| id)
                .collect();
            ids = if objects == "copy" { duplicate_in(d, sel, &on, Affine::IDENTITY)? } else { on };
            let m = old.transform_map(plane, from, Affine::IDENTITY, to - from).ok_or_else(beyond_horizon)?;
            for id in &ids {
                warp_checked(d, *id, &m)?;
                persp::set_attachment(d.node_mut(*id).ok_or(EngineError::NoNode(*id))?, plane, to);
            }
        }
        store_grid(d, &g);
        Ok(ids)
    })?;
    Ok(json!({ "plane": plane.id(), "offset": to, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

pub(super) fn plane_match(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let id = id_param(p, "id")
        .or_else(|| st.selection.objects.first().copied())
        .ok_or_else(|| EngineError::Other("select an object in perspective".into()))?;
    let mut g = grid_of(&st.doc);
    let (plane, depth) = g.attachment_of(&st.doc, id).ok_or_else(|| EngineError::Other("the object isn't in perspective".into()))?;
    g.set_offset(plane, depth);
    g.plane = plane;
    g.validate().map_err(|e| bad("perspective.plane.matchObject", e))?;
    s.edit("Move Plane to Match Object", |d, _| {
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(json!({ "plane": plane.id(), "offset": depth }))
}

pub(super) fn persp_nudge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.nudge";
    let (dx, dy) = (f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0));
    if !(dx.is_finite() && dy.is_finite()) || (dx == 0.0 && dy == 0.0) {
        return Err(bad(C, "dx and dy give the arrow's direction"));
    }
    let k = s.prefs.keyboard_increment * if bool_or(p, "big", false) { 10.0 } else { 1.0 };
    let ids = selected_roots(s)?;
    // The step is measured on the page at the centre of the selection's perspective box.
    let st = s.doc()?;
    let g = grid_of(&st.doc);
    let (plane, depth, rect) = g.plane_bounds(&st.doc, &ids).ok_or_else(|| EngineError::Other("the selection isn't in perspective".into()))?;
    let from = g.homography_at(plane, depth).and_then(|h| h.apply(rect.center())).ok_or_else(beyond_horizon)?;
    let to = from + vectorcraft_geom::Vec2::new(dx.clamp(-1.0, 1.0), dy.clamp(-1.0, 1.0)) * k;
    persp_move(
        s,
        &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "from": [from.x, from.y], "to": [to.x, to.y], "copy": bool_or(p, "copy", false)}),
    )
}

pub(super) fn persp_draw(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.draw";
    let command = str_param(p, "command").filter(|c| c.starts_with("shape.")).ok_or_else(|| bad(C, "command must be a shape.* command"))?.to_string();
    let params = p.get("params").cloned().unwrap_or_else(|| json!({}));
    let mut g = grid_of(&s.doc()?.doc);
    let plane = plane_param(&g, p, C)?;
    let snap = bool_or(p, "snap", g.snap);
    // A click-to-size dialog's shape keeps its sizes in plane units from the click.
    let map = match point_param(p, "at") {
        Some(at) => Some(g.size_homography(plane, at, snap).ok_or_else(beyond_horizon)?),
        None => None,
    };
    let undo_before = s.doc()?.history.undo.len();
    let r = s.execute(&command, &params)?;
    let id = r.get("id").and_then(Value::as_u64).map(NodeId).ok_or_else(|| EngineError::Other(format!("{command} didn't create an object")))?;
    let res = s.edit("Draw in Perspective", |d, _| {
        attach_in(d, &mut g, id, plane, snap, map)?;
        store_grid(d, &g);
        Ok(())
    });
    // One undo step for the whole gesture.
    let st = s.doc_mut()?;
    if st.interaction.is_none() && st.history.undo.len() > undo_before + 1 {
        st.history.undo.truncate(undo_before + 1);
        if let Some(e) = st.history.undo.last_mut() {
            e.label = "Draw in Perspective".into();
        }
    }
    res?;
    Ok(json!({ "id": id.0, "plane": plane.id() }))
}

/// Shape tool previews become `perspective.draw` while the grid is shown with an active plane.
pub(crate) fn perspective_rewrite(s: &Session, cmd: &str, params: &Value) -> Option<(String, Value)> {
    const SHAPES: &[&str] = &[
        "shape.rectangle",
        "shape.ellipse",
        "shape.polygon",
        "shape.star",
        "shape.line",
        "shape.rectangularGrid",
        "shape.arc",
        "shape.flare",
        "shape.spiral",
        "shape.polarGrid",
    ];
    if !SHAPES.contains(&cmd) {
        return None;
    }
    let g = PerspectiveGrid::current(&s.active()?.doc);
    (g.visible && g.plane != Plane::None).then(|| ("perspective.draw".to_string(), json!({"command": cmd, "params": params})))
}

/// A click-to-size shape dialog's command (`cmd` with `params`, clicked at `at`) as it runs: on the
/// active plane while the grid shows (`perspective.draw` keeping the sizes in plane units from the
/// click), else `None` (run it as it is).
pub fn perspective_click(s: &Session, cmd: &str, params: &Value, at: Point) -> Option<(String, Value)> {
    let (c, mut p) = perspective_rewrite(s, cmd, params)?;
    p["at"] = json!([at.x, at.y]);
    Some((c, p))
}

/// Object › Perspective › Edit Text: type in perspective shown flat where it is drawn, in isolation
/// mode, to be edited; exiting isolation projects it again ([`finish_edit_text`]).
pub(super) fn edit_text(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let id =
        id_param(p, "id").or_else(|| st.selection.objects.first().copied()).ok_or_else(|| EngineError::Other("select type in perspective".into()))?;
    if !st.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)) && n.projection().is_some()) {
        return Err(EngineError::Other("select type in perspective".into()));
    }
    s.edit("Edit Text", |d, _| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        let h = n.projection().ok_or_else(|| EngineError::Other("select type in perspective".into()))?;
        let NodeKind::Text(t) = &mut n.kind else { return Err(EngineError::Other("select type in perspective".into())) };
        // Shown flat where it is drawn: the type moves there and the projection moves the other
        // way, so the picture stays.
        let (Some(flat), Some(drawn)) = (t.bounds(), t.bounds().and_then(|b| h.map_rect_bbox(b))) else { return Ok(()) };
        let shift = vectorcraft_geom::Affine::translate(drawn.center() - flat.center());
        t.transform(shift);
        if let Some(rec) = n.perspective.as_deref_mut() {
            rec.projection = Some(h.then_after(&Homography::from_affine(shift.inverse())).to_array());
            rec.editing = true;
        }
        Ok(())
    })?;
    let st = s.doc_mut()?;
    st.isolation = Some(id);
    st.selection.set([id]);
    st.revision += 1;
    Ok(json!({ "id": id.0 }))
}

/// Edit Text ends (isolation mode on `id` exits): the type is projected again, in the document and
/// in every undo state (the flat view is never an undo step of its own).
pub(crate) fn finish_edit_text(st: &mut crate::DocState, id: NodeId) {
    let clear = |doc: &mut Arc<Document>| {
        if doc.node(id).and_then(|n| n.perspective.as_deref()).is_some_and(|p| p.editing)
            && let Some(p) = Arc::make_mut(doc).node_mut(id).and_then(|n| n.perspective.as_deref_mut())
        {
            p.editing = false;
        }
    };
    let saved = !st.is_dirty();
    clear(&mut st.doc);
    if saved {
        st.mark_saved();
    }
    for e in st.history.undo.iter_mut().chain(st.history.redo.iter_mut()) {
        clear(&mut e.doc);
    }
    if let Some(it) = &mut st.interaction {
        clear(&mut it.doc);
    }
    st.revision += 1;
}
