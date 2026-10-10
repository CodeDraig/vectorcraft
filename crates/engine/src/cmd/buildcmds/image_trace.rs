//! Image Trace creation, preview modes and expansion.

use super::super::*;
use super::{
    Appearance, AppearanceItem, Arc, Color, LineCap, LineJoin, Node, NodeId, NodeKind, Paint, StrokeLayer, TraceView, Value, fill_only, ids_json,
    json, replace_roots, root_ids, shape_node, tr,
};

/// Name of an Image Trace group.
pub const TRACE_NAME: &str = "Image Trace";

// ---------- Image Trace ----------

fn is_trace_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some(TRACE_NAME)
        && n.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
}

/// The `view` param ([`TraceView`] id), if given.
fn view_param(cmd: &str, p: &Value) -> Result<Option<TraceView>> {
    let Some(v) = p.get("view").filter(|v| !v.is_null()) else { return Ok(None) };
    let ids = || TraceView::ALL.map(TraceView::id).join(", ");
    v.as_str().and_then(TraceView::from_id).map(Some).ok_or_else(|| bad(cmd, format!("view must be one of {}", ids())))
}

fn trace_params(p: &Value) -> Result<tr::TraceParams> {
    let name = str_param(p, "preset").unwrap_or("Default");
    let base = tr::preset(name).ok_or_else(|| bad("imageTrace.make", format!("unknown preset `{name}` (see imageTrace.presets)")))?;
    let Some(over) = p.get("params").filter(|v| !v.is_null()) else { return Ok(base) };
    let mut v = serde_json::to_value(&base).map_err(|e| EngineError::Other(e.to_string()))?;
    let (Some(o), Some(src)) = (v.as_object_mut(), over.as_object()) else { return Err(bad("imageTrace.make", "params must be an object")) };
    for (k, val) in src {
        o.insert(k.clone(), val.clone());
    }
    let params: tr::TraceParams = serde_json::from_value(v).map_err(|e| bad("imageTrace.make", e.to_string()))?;
    if !params.fills && !params.strokes {
        return Err(bad("imageTrace.make", "Create needs fills, strokes or both"));
    }
    if !(params.stroke_width.is_finite() && params.stroke_width > 0.0) {
        return Err(bad("imageTrace.make", "strokeWidth must be a positive number of pixels"));
    }
    Ok(params)
}

/// Document Library in Color mode: look the library's colours up into `params.swatches` → each
/// one as traced (sRGB) with the colour to fill with.
fn trace_library(s: &Session, params: &mut tr::TraceParams) -> Result<Vec<([u8; 3], Color)>> {
    const C: &str = "imageTrace.make";
    if params.mode != tr::Mode::Color || params.palette != tr::Palette::DocumentLibrary {
        return Ok(vec![]);
    }
    let key = params.library.as_str();
    let colors = super::super::swatchlib::library_colors(s, key)
        .ok_or_else(|| bad(C, format!("no swatch library `{key}` (see swatch.library.list; \"document\": the document's swatches)")))?;
    if colors.is_empty() {
        return Err(bad(C, format!("`{key}` has no colours")));
    }
    let library: Vec<([u8; 3], Color)> = colors
        .into_iter()
        .map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            ([r, g, b], c)
        })
        .collect();
    params.swatches = library.iter().map(|l| l.0).collect();
    Ok(library)
}

/// The most anchor points an Image Trace may make: a photo traced at high fidelity can make
/// millions, more than the canvas draws smoothly or a laptop has memory for, so the trace is
/// refused with advice instead (#525).
pub const MAX_TRACE_ANCHORS: usize = 2_000_000;

pub(super) fn trace_make(s: &mut Session, p: &Value, expand: bool) -> Result<Value> {
    const C: &str = "imageTrace.make";
    let mut params = trace_params(p)?;
    let view = view_param(C, p)?;
    let params_json = serde_json::to_value(&params).map_err(|e| EngineError::Other(e.to_string()))?;
    // The preset's name if the parameters are exactly that preset's, else "Custom".
    let named = str_param(p, "preset").unwrap_or("Default");
    let preset_label = match tr::preset(named) {
        Some(b) if serde_json::to_value(&b).ok().as_ref() == Some(&params_json) => named.to_string(),
        _ => "Custom".to_string(),
    };
    let roots = root_ids(s, p)?;
    // The image (inside a trace group, or selected directly).
    let (target, img) = {
        let d = &s.doc()?.doc;
        roots
            .iter()
            .find_map(|r| {
                let n = d.node(*r)?;
                match &n.kind {
                    NodeKind::Image(i) => Some((*r, i.clone())),
                    _ if is_trace_group(n) => match &n.children()?.first()?.kind {
                        NodeKind::Image(i) => Some((*r, i.clone())),
                        _ => None,
                    },
                    _ => None,
                }
            })
            .ok_or_else(|| bad(C, "select an image (or an Image Trace group)"))?
    };
    let blob = s.doc()?.doc.images.get(&img.key).ok_or_else(|| EngineError::Other(format!("image data `{}` is missing", img.key)))?;
    let raster = tr::Raster::decode(&blob.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    let library = trace_library(s, &mut params)?;
    let res = tr::trace_within(&raster, &params, MAX_TRACE_ANCHORS).map_err(|e| EngineError::Other(e.to_string()))?;
    // Pixel space of the decoded raster → the image object's pixel space → document.
    let sx = img.width.max(1) as f64 / raster.width.max(1) as f64;
    let sy = img.height.max(1) as f64 / raster.height.max(1) as f64;
    let xf = img.xf * vectorcraft_geom::Affine::scale_non_uniform(sx, sy);
    // Document units per raster pixel (stroke widths).
    let unit = xf.determinant().abs().sqrt();
    let (npaths, anchors, colors) = (res.paths.len(), res.anchor_count(), res.palette.len());
    let label = if expand { "Image Trace (Make and Expand)" } else { "Image Trace" };
    let id = s.edit(label, |d, sel| {
        let src = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
        let mut image = if is_trace_group(&src) {
            src.children()
                .and_then(|c| c.first())
                .map(|c| (**c).clone())
                .ok_or_else(|| EngineError::Other("Image Trace: the traced image is missing".into()))?
        } else {
            src.clone()
        };
        image.visible = true;
        let mut children: Vec<Arc<Node>> = vec![];
        if !expand {
            let mut hidden = image.clone();
            hidden.visible = false;
            children.push(Arc::new(hidden));
        }
        for tp in res.paths {
            let path = tp.path.transformed(xf);
            let mut n = shape_node(d, path, None);
            // A library colour fills with the swatch's own colour (its model kept).
            let color = library.iter().find(|(rgb, _)| *rgb == tp.color).map_or_else(|| Color::rgb8(tp.color[0], tp.color[1], tp.color[2]), |l| l.1);
            let paint = Paint::solid(color);
            n.appearance = match tp.stroke {
                // Traced lines: round ends and corners, as drawn lines have.
                Some(width) => Appearance {
                    items: vec![AppearanceItem::Stroke(StrokeLayer {
                        cap: LineCap::Round,
                        join: LineJoin::Round,
                        ..StrokeLayer::new(paint, width * unit)
                    })],
                    ..Default::default()
                },
                None => fill_only(paint),
            };
            children.push(Arc::new(n));
        }
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = if expand { None } else { Some(TRACE_NAME.into()) };
        if !expand {
            // Tracing again keeps the object's view unless another is asked for.
            let view = view.unwrap_or_else(|| src.trace_view());
            g.trace = Some(Box::new(json!({ "preset": preset_label, "params": params_json, "view": view.id() })));
        }
        let (par, idx, _) = d.position(target).ok_or(EngineError::NoNode(target))?;
        d.remove(target)?;
        d.insert(par, idx, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "paths": npaths, "anchors": anchors, "colors": colors }))
}

fn trace_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let v: Vec<NodeId> = root_ids(s, p)?.into_iter().filter(|i| d.node(*i).is_some_and(is_trace_group)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select an Image Trace object".into()));
    }
    Ok(v)
}

pub(super) fn trace_set_view(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "imageTrace.setView";
    let view = view_param(C, p)?.ok_or_else(|| bad(C, "view is required"))?;
    let targets = trace_targets(s, p)?;
    let d = &s.doc()?.doc;
    if targets.iter().all(|g| d.node(*g).is_some_and(|n| n.trace_view() == view)) {
        return Ok(json!({ "ids": ids_json(&targets), "view": view.id() }));
    }
    s.edit("Image Trace View", |d, _| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            if let Some(t) = n.trace.as_deref_mut().and_then(Value::as_object_mut) {
                t.insert("view".into(), json!(view.id()));
            } else {
                n.trace = Some(Box::new(json!({ "preset": "Custom", "view": view.id() })));
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets), "view": view.id() }))
}

pub(super) fn trace_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = trace_targets(s, p)?;
    let ids = s.edit("Release Image Trace", |d, sel| {
        let mut out = vec![];
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let mut img = (**n.children().and_then(|c| c.first()).ok_or(EngineError::NoNode(*g))?).clone();
            img.visible = true;
            out.extend(replace_roots(d, &[*g], vec![img])?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

pub(super) fn trace_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = trace_targets(s, p)?;
    s.edit("Expand Image Trace", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            n.trace = None;
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| !matches!(c.kind, NodeKind::Image(_)));
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}
