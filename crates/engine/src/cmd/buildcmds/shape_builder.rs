//! Shape Builder region merging and replacement.

use super::super::*;
use super::{
    BuilderMap, FillRule, Node, PathData, Rect, SubPath, Touched, Value, b, fill_param, finite, ids_json, json, po, points_param, replace_roots,
    root_ids, sample_polyline, shape_node,
};

pub(super) fn sb_merge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shapeBuilder.merge";
    let pts = points_param(p, "points").ok_or_else(|| bad(C, "missing points [[x,y]…]"))?;
    if pts.is_empty() || !pts.iter().all(|q| finite(*q)) {
        return Err(bad(C, "points must be a non-empty list of finite [x,y]"));
    }
    let erase = bool_or(p, "erase", false);
    // Line pieces and edges are only deleted (erase mode), never merged.
    let piece_tol = erase.then(|| f64_or(p, "tolerance", 2.0).clamp(0.0, 1000.0));
    let fill = fill_param(s, p, C)?;
    let current_fill = s.paint.fill.clone();
    let roots = root_ids(s, p)?;
    let label = if erase { "Shape Builder Delete" } else { "Shape Builder" };
    let (ids, merged, touched) = s.edit(label, |d, sel| {
        let (map, leaves) = BuilderMap::new(d, &roots, erase);
        let leaves: Vec<Node> = leaves.into_iter().cloned().collect();
        if map.shapes.is_empty() {
            return Err(EngineError::Other("Shape Builder: select paths or compound paths".into()));
        }
        let step = (d.bounds_of(&roots, false).map(|r| r.width().max(r.height())).unwrap_or(100.0) / 2000.0).max(0.25);
        let mut touched = Touched::default();
        for q in sample_polyline(&pts, step) {
            touched.add(map.hit(q, piece_tol));
        }
        touched.edges = map.erased_edges(&touched);
        if touched.is_empty() {
            return Err(EngineError::Other("Shape Builder: no region under the pointer".into()));
        }
        let cuts: Vec<&SubPath> = touched.edges.iter().filter_map(|&k| map.edges.get(k)?.0.path.subpaths.first()).collect();
        // Edges come back from the planar map within a few hundredths of the outlines they follow.
        let cut_tol = step * 0.2;
        let picked: Vec<&po::Region> = touched.regions.iter().filter_map(|&i| map.regions.get(i).map(|r| &r.0)).collect();
        let faces = po::FaceMerger::new(&map.shapes);
        let union = if picked.is_empty() { PathData::default() } else { faces.merge(&picked) };
        // The merged shape takes the look of the front-most object over the first region, or (for an
        // area only lines enclose) of the front-most object and the current fill.
        let covered = picked.iter().find_map(|r| r.top());
        let style_src = covered.or(map.shapes.len().checked_sub(1));
        let merged_fill = fill.or_else(|| covered.is_none().then_some(current_fill));
        let mut new_nodes: Vec<Node> = vec![];
        let mut merged_id = None;
        for (i, (sh, leaf)) in map.shapes.iter().zip(&leaves).enumerate() {
            let pieces: Vec<usize> = map.lines.iter().enumerate().filter(|(_, (l, _))| l.key == i as u64).map(|(k, _)| k).collect();
            if !pieces.is_empty() {
                // An open path loses the pieces deleted and those between merged (or deleted) regions.
                let keep: Vec<usize> =
                    pieces.iter().copied().filter(|k| !touched.lines.contains(k) && !map.line_between(*k, &touched.regions)).collect();
                if keep.len() == pieces.len() {
                    new_nodes.push(d.reid(leaf));
                } else {
                    for k in keep {
                        if let Some((l, _)) = map.lines.get(k) {
                            new_nodes.push(shape_node(d, l.path.clone(), Some(leaf)));
                        }
                    }
                }
            } else {
                // A shape loses the regions deleted or merged, then the edges deleted (each one on
                // its outline, shared edges from every shape along them).
                let in_union = picked.iter().any(|r| r.sources.contains(&i));
                let mut rest = if in_union {
                    // What is left of it: its faces not merged or deleted, united. They share their
                    // edges with the merged faces exactly, where subtracting the merged shape left
                    // slivers of the outline along them (#569).
                    let kept: Vec<&po::Region> = map
                        .regions
                        .iter()
                        .enumerate()
                        .filter(|(k, (r, ..))| r.sources.contains(&i) && !touched.regions.contains(k))
                        .map(|(_, (r, ..))| r)
                        .collect();
                    if kept.is_empty() { PathData::default() } else { faces.merge(&kept) }
                } else {
                    b::node_outline(leaf).map_or_else(|| sh.path.clone(), |o| o.0)
                };
                let fill_closes = !leaf.appearance.fill_paint().is_none();
                let mut cut = false;
                for piece in &cuts {
                    if let Some(r) = po::cut_out(&rest, piece, fill_closes, cut_tol) {
                        rest = r;
                        cut = true;
                    }
                }
                if !in_union && !cut {
                    new_nodes.push(d.reid(leaf));
                } else if !rest.is_empty() {
                    new_nodes.push(shape_node(d, rest, Some(leaf)));
                }
            }
            if !erase && !union.is_empty() && Some(i) == style_src {
                let mut m = shape_node(d, union.clone(), Some(leaf));
                if let Some(f) = &merged_fill {
                    m.appearance.set_fill(f.clone());
                }
                merged_id = Some(m.id);
                new_nodes.push(m);
            }
        }
        let new_ids = replace_roots(d, &roots, new_nodes)?;
        sel.set(new_ids.iter().copied());
        Ok((new_ids, merged_id, touched))
    })?;
    Ok(json!({
        "ids": ids_json(&ids),
        "merged": merged.map(|m| m.0),
        "regions": touched.regions.len(),
        "lines": touched.lines.len(),
        "edges": touched.edges.len(),
    }))
}

pub(super) fn sb_regions(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let d = &s.doc()?.doc;
    let (map, leaves) = BuilderMap::new(d, &roots, true);
    let id_of = |i: usize| leaves.get(i).map(|n| n.id.0);
    let regions: Vec<Value> = map
        .regions
        .iter()
        .map(|(r, _, bb)| {
            json!({
                "bounds": [bb.x0, bb.y0, bb.x1, bb.y1],
                "area": po::area(&r.path, FillRule::NonZero),
                "sources": r.sources.iter().filter_map(|&i| id_of(i)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let pieces = |list: &[(po::Shape, Rect)]| -> Vec<Value> {
        list.iter().map(|(l, bb)| json!({ "bounds": [bb.x0, bb.y0, bb.x1, bb.y1], "source": id_of(l.key as usize) })).collect()
    };
    Ok(json!({ "regions": regions, "lines": pieces(&map.lines), "edges": pieces(&map.edges) }))
}
