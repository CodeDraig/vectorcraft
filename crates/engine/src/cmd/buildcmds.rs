//! Shape Builder (`shapeBuilder.*`), Live Paint (`livePaint.*`) and Image Trace (`imageTrace.*`).
//!
//! **Live Paint groups** need no special node kind: a Live Paint group is a plain group whose name
//! starts with "Live Paint". Its first child is a hidden group named "Live Paint Sources" holding
//! the original paths (the planar map's inputs, used by Release, Merge and re-computation); then
//! come the faces (paths named "Face", fill only) and the edges (open paths named "Edge", stroke
//! only). See `vectorcraft_tools::builder` for the shared helpers.
//!
//! **Image Trace** makes a group named "Image Trace" holding the source image (hidden, bottom)
//! and the traced filled paths above it, mapped with the image's transform. Expand drops the
//! image (a plain group of paths remains); Release puts the image back in place of the group.

mod shape_builder;
use shape_builder::{sb_merge, sb_regions};
mod live_paint;
use live_paint::{lp_expand, lp_fill, lp_info, lp_make, lp_merge, lp_release, lp_stroke_edge};
mod image_trace;
use image_trace::{trace_expand, trace_make, trace_release, trace_set_view};

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer, LineCap, LineJoin, StrokeLayer};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Selection, TraceView};
use vectorcraft_geom::{FillRule, PathData, Point, Rect, SubPath};
use vectorcraft_pathops as po;
use vectorcraft_tools::builder::{
    self as b, BuilderMap, EDGE_NAME, FACE_NAME, LIVE_PAINT_NAME, SOURCES_NAME, Touched, edge_near, face_at, is_live_paint, sample_polyline,
    sorted_roots,
};
use vectorcraft_trace as tr;

use super::edit::selected_roots;
use super::paint::paint_from;
use super::pathops::shape_node;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "shapeBuilder.merge",
            "Shape Builder",
            [],
            None,
            "{ids?: [id…] (default: selection), points: [[x,y]…] (drag path; one point = click), erase?: bool (erase mode, Alt: deletes the regions touched, and the pieces of unfilled open paths and the edges (outline pieces between the points where paths meet; see shapeBuilder.regions) within tolerance; a deleted edge opens its path there, and edges bounding a deleted region go with it), tolerance?: pt (2, erase only), fill?: \"#rrggbb\" | {color|swatch|gradient|none} (default: the fill of the object under the first point; the current fill for an area only lines enclose)} → {ids, merged, regions, lines, edges} (counts deleted or merged)",
            has_selection_or_ids,
            sb_merge
        ),
        cmd!(
            query "shapeBuilder.regions",
            "Shape Builder Regions",
            [],
            None,
            "{ids?} → {regions: [{bounds: [x0,y0,x1,y1], area, sources: [id…]}], lines: [{bounds, source: id}], edges: [{bounds, source: id}]} faces of the planar arrangement (unfilled open paths cut them; sources is empty for an area only they enclose), the pieces of those open paths and the edges (the other paths' outlines cut where paths meet; source: the front-most path along it), which erasing deletes",
            has_selection_or_ids,
            sb_regions
        ),
        cmd!(
            "livePaint.make",
            "Make",
            ["Object", "Live Paint"],
            Some("Cmd+Alt+X"),
            "{ids?} build a Live Paint group from the selected paths → {id, faces, edges}",
            has_selection_or_ids,
            lp_make
        ),
        cmd!(
            "livePaint.merge",
            "Merge",
            ["Object", "Live Paint"],
            None,
            "{ids?} add the other selected paths (and Live Paint groups) to the first Live Paint group, keeping its paint → {id, faces, edges}",
            has_multi_or_ids,
            lp_merge
        ),
        cmd!(
            "livePaint.release",
            "Release",
            ["Object", "Live Paint"],
            None,
            "{id?} replace Live Paint groups with their original paths → {ids}",
            has_selection_or_ids,
            lp_release
        ),
        cmd!(
            "livePaint.expand",
            "Expand",
            ["Object", "Live Paint"],
            None,
            "{id?} turn Live Paint groups into plain groups of their painted faces and edges → {ids}",
            has_selection_or_ids,
            lp_expand
        ),
        cmd!(
            "livePaint.fill",
            "Live Paint Bucket",
            [],
            None,
            "{group?: id | ids?: [id…] (paths to make live first), point: [x,y], color?|swatch?|gradient?|none? (default: current fill)} fill the face containing point → {group, face}",
            has_doc,
            lp_fill
        ),
        cmd!(
            "livePaint.strokeEdge",
            "Live Paint Bucket (Stroke)",
            [],
            None,
            "{group: id, point: [x,y], color?|swatch?|none? (default: current stroke), width?: pt (default: current weight), tolerance?: pt (4)} paint the edge nearest point → {group, edge}",
            has_doc,
            lp_stroke_edge
        ),
        cmd!(
            query "livePaint.info",
            "Live Paint Info",
            [],
            None,
            "{group} → {faces: [{id, fill, bounds}], edges: [{id, stroke, width}], sources}",
            has_doc,
            lp_info
        ),
        cmd!(
            "imageTrace.make",
            "Make",
            ["Object", "Image Trace"],
            None,
            "{id?: image or Image Trace group (default: selection), preset?: name (imageTrace.presets), params?: {mode: \"blackAndWhite\"|\"grayscale\"|\"color\"|\"logo\", threshold: 0-255, palette: \"limited\"|\"fullTone\"|\"automatic\"|\"documentLibrary\" (color mode; limited: the `colors` best colours, fullTone: as many as the tones need, more with colorDetail, automatic: flat art with its own colours, else fullTone, documentLibrary: at most `colors` of the library's colours, as they are), colors: 2-256, colorDetail: 0-100 (fullTone, automatic), library: \"document\" (the document's swatches) or a swatch library id or name (swatch.library.list; documentLibrary), paths: 0-100, corners: 0-100, noise: px, method: \"abutting\"|\"overlapping\", ignoreWhite, snapCurvesToLines, fills: true, strokes: false (Create: lines up to strokeWidth wide become stroked centre lines; without fills, wider areas become stroked outlines), strokeWidth: px (10), logoColors?: [\"#rrggbb\"…] (mode logo: flat-colour logos only; colours found automatically or named here, white is always its own colour, at most ten; noise = areas under noise/10 px ignored; edges to a fraction of a pixel, sharp corners restored, lines and circles snapped; enclosed transparent areas are painted white unless ignoreWhite makes them holes; fills/strokes are ignored; gradients and photos are refused)}, view?: as imageTrace.setView (default: the traced object's own, else tracingResult)} → {id, paths, anchors, colors}",
            has_selection_or_ids,
            |s, p| trace_make(s, p, false)
        ),
        cmd!(
            "imageTrace.makeAndExpand",
            "Make and Expand",
            ["Object", "Image Trace"],
            None,
            "same as imageTrace.make, then expand → {id, paths, anchors, colors}",
            has_selection_or_ids,
            |s, p| trace_make(s, p, true)
        ),
        cmd!(
            "imageTrace.release",
            "Release",
            ["Object", "Image Trace"],
            None,
            "{id?} put the source image back in place of Image Trace groups → {ids}",
            has_selection_or_ids,
            trace_release
        ),
        cmd!(
            "imageTrace.expand",
            "Expand",
            ["Object", "Image Trace"],
            None,
            "{id?} keep only the traced paths (drop the source image) → {ids}",
            has_selection_or_ids,
            trace_expand
        ),
        cmd!(
            "imageTrace.setView",
            "Image Trace View",
            [],
            None,
            "{id?|ids?: Image Trace objects (default: selection), view: \"tracingResult\"|\"tracingResultWithOutlines\"|\"outlines\"|\"outlinesWithSourceImage\"|\"sourceImage\"} how they draw on screen; exports, printing and Expand keep the tracing result → {ids, view}",
            has_selection_or_ids,
            trace_set_view
        ),
        cmd!(query "imageTrace.presets", "Image Trace Presets", [], None, "{} → {presets: [{name, params}]}", always, |_, _| {
            Ok(json!({ "presets": tr::presets().into_iter().map(|(n, p)| json!({ "name": n, "params": p })).collect::<Vec<_>>() }))
        }),
    ]
}

fn has_selection_or_ids(s: &Session) -> std::result::Result<(), String> {
    // Agents may pass ids explicitly; menus need a selection. Accept either (params are checked in run).
    has_doc(s)
}

fn has_multi_or_ids(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)
}

fn root_ids(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let ids = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    if ids.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let d = &s.doc()?.doc;
    if let Some(bad_id) = ids.iter().find(|i| d.node(**i).is_none()) {
        return Err(EngineError::NoNode(*bad_id));
    }
    // Layers are containers, not objects.
    let ids: Vec<NodeId> = ids.into_iter().filter(|i| d.node(*i).is_some_and(|n| !n.is_layer())).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("select objects, not layers".into()));
    }
    Ok(sorted_roots(d, &ids))
}

fn points_param(p: &Value, key: &str) -> Option<Vec<Point>> {
    p.get(key)?.as_array()?.iter().map(|v| Some(Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))).collect()
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= crate::MAX_COORD && p.y.abs() <= crate::MAX_COORD
}

fn fill_only(paint: Paint) -> Appearance {
    Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint))], ..Default::default() }
}

fn stroke_only(paint: Paint, width: f64) -> Appearance {
    Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(paint, width))], ..Default::default() }
}

// ---------- Shape Builder ----------

fn fill_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<Paint>> {
    match p.get("fill") {
        None | Some(Value::Null) => Ok(None),
        Some(v @ Value::Object(o)) if o.contains_key("color") || o.contains_key("swatch") || o.contains_key("gradient") || o.contains_key("none") => {
            paint_from(s, v)
        }
        Some(v) => color_value(v).map(|c| Some(Paint::solid(c))).ok_or_else(|| bad(cmd, format!("bad fill {v}"))),
    }
}

/// Remove the roots and insert `nodes` where the front-most root was. Returns the new ids.
/// (Roots go first: the new nodes may reuse their ids, e.g. Live Paint sources.)
fn replace_roots(d: &mut Document, roots: &[NodeId], nodes: Vec<Node>) -> Result<Vec<NodeId>> {
    let top = *roots.last().ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
    let before = roots.iter().filter(|r| **r != top && d.position(**r).is_some_and(|(p, i, _)| p == par && i < idx)).count();
    for r in roots {
        if d.node(*r).is_some() {
            d.remove(*r)?;
        }
    }
    let at = idx - before;
    let mut ids = vec![];
    for (k, n) in nodes.into_iter().enumerate() {
        ids.push(d.insert(par, at + k, n)?);
    }
    Ok(ids)
}
