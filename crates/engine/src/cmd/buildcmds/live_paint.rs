//! Live Paint source geometry, faces, edges and expansion.

use super::super::*;
use super::{
    Appearance, Arc, Document, EDGE_NAME, FACE_NAME, LIVE_PAINT_NAME, Node, NodeId, Paint, Point, Rect, SOURCES_NAME, Selection, Value, b, edge_near,
    face_at, fill_only, finite, ids_json, is_live_paint, json, paint_from, po, replace_roots, root_ids, stroke_only,
};

// ---------- Live Paint ----------

/// The source nodes a root contributes (a Live Paint group contributes its own sources).
fn source_nodes(n: &Node) -> Vec<Arc<Node>> {
    if is_live_paint(n) { b::sources(n).and_then(|g| g.children().cloned()).unwrap_or_default() } else { vec![Arc::new(n.clone())] }
}

/// Planar map children (sources group, faces, edges) for `sources`.
fn build_children(d: &mut Document, sources: Vec<Arc<Node>>) -> Result<(Vec<Arc<Node>>, usize, usize)> {
    let mut lv = vec![];
    for n in &sources {
        b::leaf_shapes(n, &mut lv);
    }
    let mut shapes = vec![];
    let mut styles: Vec<(Paint, Paint, f64)> = vec![];
    for l in lv {
        if let Some((path, rule)) = b::node_outline(l)
            && !path.is_empty()
        {
            shapes.push(po::Shape::new(path, rule, styles.len() as u64));
            let a = &l.appearance;
            styles.push((a.fill_paint(), a.stroke_paint(), a.stroke().map(|s| s.width).unwrap_or(1.0)));
        }
    }
    if shapes.is_empty() {
        return Err(EngineError::Other("Live Paint: select paths or compound paths".into()));
    }
    let mut children: Vec<Arc<Node>> = vec![];
    let gid = d.alloc_id();
    let mut src_group = Node::group(gid, sources);
    src_group.name = Some(SOURCES_NAME.into());
    src_group.visible = false;
    children.push(Arc::new(src_group));
    // Every path is an edge, open ones included; faces are the areas they enclose.
    let (regs, edges) = po::live_paint(&shapes);
    if regs.is_empty() && edges.is_empty() {
        return Err(EngineError::Other("Live Paint: these paths are too degenerate to paint".into()));
    }
    let nfaces = regs.len();
    for r in regs {
        let id = d.alloc_id();
        let fill = r.top().and_then(|t| styles.get(t)).map_or(Paint::None, |s| s.0.clone());
        let mut n = Node::path(id, r.path, fill_only(fill));
        n.name = Some(FACE_NAME.into());
        children.push(Arc::new(n));
    }
    let nedges = edges.len();
    for e in edges {
        let Some((_, stroke, w)) = styles.get(e.key as usize) else { continue };
        let id = d.alloc_id();
        let mut n = Node::path(id, e.path, stroke_only(stroke.clone(), *w));
        n.name = Some(EDGE_NAME.into());
        children.push(Arc::new(n));
    }
    Ok((children, nfaces, nedges))
}

/// Make a Live Paint group from `roots`; returns (group, faces, edges).
fn make_live(d: &mut Document, sel: &mut Selection, roots: &[NodeId]) -> Result<(NodeId, usize, usize)> {
    let mut sources = vec![];
    for r in roots {
        let n = d.node(*r).ok_or(EngineError::NoNode(*r))?;
        sources.extend(source_nodes(n));
    }
    let (children, nf, ne) = build_children(d, sources)?;
    let gid = d.alloc_id();
    let mut g = Node::group(gid, children);
    g.name = Some(LIVE_PAINT_NAME.into());
    replace_roots(d, roots, vec![g])?;
    sel.set([gid]);
    Ok((gid, nf, ne))
}

pub(super) fn lp_make(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let (id, faces, edges) = s.edit("Make Live Paint", |d, sel| make_live(d, sel, &roots))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

pub(super) fn lp_merge(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let (id, faces, edges) = s.edit("Merge Live Paint", |d, sel| {
        let target = roots
            .iter()
            .copied()
            .find(|r| d.node(*r).is_some_and(is_live_paint))
            .ok_or_else(|| EngineError::Other("Live Paint Merge: select a Live Paint group and the paths to add".into()))?;
        let old = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
        // Paint to carry over: (point inside face, fill) and (point on edge, stroke).
        let keep_faces: Vec<(Point, Paint)> = b::faces(&old)
            .iter()
            .filter(|f| !f.appearance.fill_paint().is_none())
            .filter_map(|f| Some((po::interior_point(f.path_data()?)?, f.appearance.fill_paint())))
            .collect();
        let keep_edges: Vec<(Point, Appearance)> = b::edges(&old)
            .iter()
            .filter_map(|e| {
                let pd = e.path_data()?;
                let sp = pd.subpaths.first().filter(|sp| sp.segment_count() > 0)?;
                Some((kurbo::ParamCurve::eval(&sp.segment(0), 0.5), e.appearance.clone()))
            })
            .collect();
        let mut sources = source_nodes(&old);
        for r in &roots {
            if *r != target {
                sources.extend(source_nodes(d.node(*r).ok_or(EngineError::NoNode(*r))?));
            }
        }
        let (children, nf, ne) = build_children(d, sources)?;
        let mut g = Node::group(target, children);
        g.name = old.name.clone();
        g.opacity = old.opacity;
        g.blend = old.blend;
        for (pt, paint) in keep_faces {
            if let Some(fid) = face_at(&g, pt).map(|f| f.id)
                && let Some(ch) = g.children_mut()
                && let Some(f) = ch.iter_mut().find(|c| c.id == fid)
            {
                Arc::make_mut(f).appearance.set_fill(paint);
            }
        }
        for (pt, app) in keep_edges {
            if let Some(eid) = edge_near(&g, pt, 0.5).map(|e| e.id)
                && let Some(ch) = g.children_mut()
                && let Some(e) = ch.iter_mut().find(|c| c.id == eid)
            {
                Arc::make_mut(e).appearance = app;
            }
        }
        for r in roots.iter().filter(|r| **r != target) {
            if d.node(*r).is_some() {
                d.remove(*r)?;
            }
        }
        let (par, idx, _) = d.position(target).ok_or(EngineError::NoNode(target))?;
        d.remove(target)?;
        d.insert(par, idx, g)?;
        sel.set([target]);
        Ok((target, nf, ne))
    })?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

fn live_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids = root_ids(s, p)?;
    let v: Vec<NodeId> = ids.into_iter().filter(|i| d.node(*i).is_some_and(is_live_paint)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select a Live Paint group".into()));
    }
    Ok(v)
}

pub(super) fn lp_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    let ids = s.edit("Release Live Paint", |d, sel| {
        let mut out = vec![];
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let srcs: Vec<Node> = source_nodes(&n).iter().map(|c| (**c).clone()).collect();
            out.extend(replace_roots(d, &[*g], srcs)?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

pub(super) fn lp_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    s.edit("Expand Live Paint", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            n.trace = None;
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| match c.name.as_deref() {
                    Some(SOURCES_NAME) => false,
                    Some(FACE_NAME) => !c.appearance.fill_paint().is_none(),
                    Some(EDGE_NAME) => !c.appearance.stroke_paint().is_none() && c.appearance.stroke_width() > 0.0,
                    _ => true,
                });
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}

pub(super) fn lp_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.fill";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.fill.clone());
    let group = id_param(p, "group");
    let roots = if group.is_none() { root_ids(s, p)? } else { vec![] };
    let (g, face) = s.edit("Live Paint Bucket", |d, sel| {
        let gid = match group {
            Some(g) => {
                if !d.node(g).is_some_and(is_live_paint) {
                    return Err(bad(C, format!("{g} is not a Live Paint group")));
                }
                g
            }
            None => match roots.iter().copied().find(|r| d.node(*r).is_some_and(|n| is_live_paint(n) && face_at(n, pt).is_some())) {
                Some(g) => g,
                None => make_live(d, sel, &roots)?.0,
            },
        };
        let fid =
            d.node(gid).and_then(|n| face_at(n, pt)).map(|f| f.id).ok_or_else(|| EngineError::Other("no Live Paint face at that point".into()))?;
        d.node_mut(fid).ok_or(EngineError::NoNode(fid))?.appearance.set_fill(paint.clone());
        Ok((gid, fid))
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "face": face.0 }))
}

pub(super) fn lp_stroke_edge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.strokeEdge";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let g = id_param(p, "group").ok_or_else(|| bad(C, "missing group"))?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.stroke.clone());
    let width = p.get("width").and_then(Value::as_f64).unwrap_or(s.paint.stroke_width.max(0.0));
    if !(0.0..=1000.0).contains(&width) {
        return Err(bad(C, "width out of range"));
    }
    let tol = f64_or(p, "tolerance", 4.0).clamp(0.0, 1000.0);
    let edge = s.edit("Live Paint Bucket", |d, _| {
        let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad(C, format!("{g} is not a Live Paint group")))?;
        let eid = edge_near(n, pt, tol).map(|e| e.id).ok_or_else(|| EngineError::Other("no Live Paint edge near that point".into()))?;
        let e = d.node_mut(eid).ok_or(EngineError::NoNode(eid))?;
        e.appearance.set_stroke(paint.clone());
        if let Some(st) = e.appearance.stroke_mut() {
            st.width = width;
        }
        Ok(eid)
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "edge": edge.0 }))
}

pub(super) fn lp_info(s: &mut Session, p: &Value) -> Result<Value> {
    let g = id_param(p, "group").or_else(|| id_param(p, "id")).ok_or_else(|| bad("livePaint.info", "missing group"))?;
    let d = &s.doc()?.doc;
    let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad("livePaint.info", format!("{g} is not a Live Paint group")))?;
    let faces: Vec<Value> = b::faces(n)
        .iter()
        .map(|f| {
            let bb = f.geometric_bounds().unwrap_or(Rect::ZERO);
            json!({ "id": f.id.0, "fill": f.appearance.fill_paint().label(), "bounds": [bb.x0, bb.y0, bb.x1, bb.y1] })
        })
        .collect();
    let edges: Vec<Value> = b::edges(n)
        .iter()
        .map(|e| json!({ "id": e.id.0, "stroke": e.appearance.stroke_paint().label(), "width": e.appearance.stroke_width() }))
        .collect();
    let sources = b::sources(n).and_then(|g| g.children()).map(|c| c.len()).unwrap_or(0);
    Ok(json!({ "faces": faces, "edges": edges, "sources": sources }))
}
