//! Puppet Warp deformation and pin state.

use super::super::*;
use super::{
    Arc, EngineError, NodeId, PinSet, Point, PuppetPin, Value, arap, json, mesh_for, points_of, selected_roots, warp_from_rest, warp_node_with,
};

// ---------- puppet warp ----------

/// The objects a Puppet Warp command works on: `id`/`ids`, else `default`.
fn puppet_ids(p: &Value, default: impl FnOnce() -> Result<Vec<NodeId>>) -> Result<Vec<NodeId>> {
    let ids = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => default()?,
    };
    if ids.is_empty() {
        return Err(EngineError::Other("select the artwork to warp".into()));
    }
    Ok(ids)
}

pub(super) fn puppet_warp(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.puppetWarp";
    let pins = points_of(p, "pins").ok_or_else(|| bad(C, "pins must be [[x,y]…]"))?;
    let moved = points_of(p, "moved").ok_or_else(|| bad(C, "moved must be [[x,y]…]"))?;
    let rest = bool_or(p, "rest", false);
    if (pins.is_empty() && !rest) || pins.len() != moved.len() {
        return Err(bad(C, "pins and moved must be non-empty lists of the same length"));
    }
    if pins.iter().chain(&moved).any(|q| !q.x.is_finite() || !q.y.is_finite()) {
        return Err(bad(C, "pins must be finite"));
    }
    let ids = if rest { puppet_ids(p, || Ok(s.doc()?.selection.objects.clone()))? } else { puppet_ids(p, || selected_roots(s))? };
    let expand = f64_or(p, "expand", 3.0).clamp(0.0, 1000.0);
    let angles: Vec<Option<f64>> = match p.get("angles") {
        None | Some(Value::Null) => vec![None; pins.len()],
        Some(Value::Array(a)) if a.len() == pins.len() => a.iter().map(|v| v.as_f64().filter(|d| d.is_finite()).map(f64::to_radians)).collect(),
        Some(_) => return Err(bad(C, "angles must be a list as long as pins (degrees or null)")),
    };
    let ids_json = json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() });
    if rest {
        let pins: Vec<PuppetPin> = pins.iter().zip(&moved).zip(angles).map(|((r, a), angle)| PuppetPin { rest: *r, at: *a, angle }).collect();
        s.edit("Puppet Warp", |d, _| warp_from_rest(d, &ids, pins, expand).map_err(|e| bad(C, e)))?;
        return Ok(ids_json);
    }
    let mesh = mesh_for(&s.doc()?.doc, &ids, expand).ok_or_else(|| EngineError::Other("nothing to warp".into()))?;
    let pins: Vec<arap::Pin> = pins.iter().zip(&moved).zip(angles).map(|((a, b), angle)| arap::Pin { angle, ..arap::Pin::new(*a, *b) }).collect();
    let deformed = arap::deform(&mesh, &pins);
    let f = |q: Point| mesh.map(&deformed, q);
    s.edit("Puppet Warp", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            warp_node_with(n, &f);
        }
        Ok(())
    })?;
    Ok(ids_json)
}

/// Forget the Puppet Warp pins: not an undo step, and a saved document stays saved (pins are
/// never saved).
pub(crate) fn drop_puppet_pins(st: &mut crate::DocState) {
    let clean = !st.is_dirty();
    Arc::make_mut(&mut st.doc).puppet = None;
    if clean {
        st.mark_saved();
    }
}

pub(super) fn puppet_pins(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = puppet_ids(p, || Ok(s.doc()?.selection.objects.clone()))?;
    let expand = f64_or(p, "expand", 3.0).clamp(0.0, 1000.0);
    let set = PinSet::of(&s.doc()?.doc, &ids, expand).ok_or_else(|| EngineError::Other("nothing to warp".into()))?;
    let mut v = set.params(&set.pins, expand);
    v["auto"] = json!(set.auto);
    Ok(v)
}
