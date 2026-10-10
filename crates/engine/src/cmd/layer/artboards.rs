//! Artboard creation, geometry, deletion and fitting.

use super::super::*;
use super::{Affine, Artboard, EngineError, Rect, Scaling, Value, json};

fn rect_from(p: &Value, default: Rect) -> Rect {
    let x = f64_or(p, "x", default.x0);
    let y = f64_or(p, "y", default.y0);
    let w = f64_or(p, "width", default.width()).max(1.0);
    let h = f64_or(p, "height", default.height()).max(1.0);
    Rect::new(x, y, x + w, y + h)
}

pub(super) fn artboard_new(s: &mut Session, p: &Value) -> Result<Value> {
    let d0 = &s.doc()?.doc;
    let last = d0.artboards.last().map(|a| a.rect).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
    let def = Rect::new(last.x1 + 20.0, last.y0, last.x1 + 20.0 + last.width(), last.y1);
    let r = rect_from(p, def);
    let idx = s.edit("New Artboard", |d, _| {
        let id = d.next_artboard_id();
        let name = str_param(p, "name").map(str::to_string).unwrap_or_else(|| format!("Artboard {}", d.artboards.len() + 1));
        d.artboards.push(Artboard { id, name, rect: r, show_center_mark: false, show_cross_hairs: false });
        Ok(d.artboards.len() - 1)
    })?;
    Ok(json!({ "index": idx }))
}

pub(super) fn artboard_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("artboard.delete", "missing index"))? as usize;
    s.edit("Delete Artboard", |d, sel| {
        if d.artboards.len() <= 1 {
            return Err(EngineError::Other("a document needs at least one artboard".into()));
        }
        if i >= d.artboards.len() {
            return Err(EngineError::Other("no such artboard".into()));
        }
        // Its guides go with it.
        let id = d.artboards.remove(i).id;
        d.retain_guides(sel, |_, g| g.artboard != Some(id));
        Ok(())
    })?;
    ok()
}

pub(super) fn artboard_set(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let was = s.doc()?.doc.artboards.get(i).map(|a| a.rect).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    let now = rect_from(p, was);
    let resized = (now.width() - was.width()).abs() > 1e-6 || (now.height() - was.height()).abs() > 1e-6;
    // Scale Artwork with Artboard: resized, it takes the art fully inside it (as Move Artwork with
    // Artboard gathers it) and its guides from the old rectangle onto the new one.
    let scale_art = bool_or(p, "scaleArt", false);
    let fit = if scale_art && resized { rect_map(was, now) } else { None };
    let art = if fit.is_some() {
        // Locked and hidden art too? Noted in the journal, so a replay scales the same objects
        // whatever the preference is then.
        let all = bool_or(p, "lockedAndHidden", s.prefs.move_locked_with_artboard);
        s.note_journal("lockedAndHidden", json!(all));
        s.doc()?.doc.art_on_artboard(was, all)
    } else {
        vec![]
    };
    let mut sc = match fit {
        Some(xf) if Scaling::factor(xf).is_some() => super::super::object::scaling(s, p),
        _ => Scaling::default(),
    };
    sc.patterns = super::super::object::transform_patterns(s, p, &art)?;
    s.edit("Artboard Options", |d, _| {
        let a = d.artboards.get_mut(i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
        a.rect = now;
        if let Some(n) = str_param(p, "name") {
            a.name = n.to_string();
        }
        let id = a.id;
        if let Some(xf) = fit {
            d.map_artboard_guides(id, xf);
            for id in &art {
                if let Some(n) = d.node_mut(*id) {
                    n.transform(xf, sc);
                }
            }
        } else if !resized {
            // Moved (not resized), it takes its guides along.
            d.move_artboard_guides(id, now.origin() - was.origin());
        }
        Ok(())
    })?;
    if !scale_art {
        return ok();
    }
    Ok(json!({ "scaled": art.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// The map taking artboard rectangle `from` onto `to`, each side scaled by its own ratio. None
/// when `from` has no area or the map isn't finite (a size out of range).
fn rect_map(from: Rect, to: Rect) -> Option<Affine> {
    if from.width() <= 1e-9 || from.height() <= 1e-9 {
        return None;
    }
    let (sx, sy) = (to.width() / from.width(), to.height() / from.height());
    let xf = Affine::translate(to.origin().to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-from.origin().to_vec2());
    xf.is_finite().then_some(xf)
}

pub(super) fn artboard_fit_art(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let b = s.doc()?.doc.art_bounds().ok_or_else(|| EngineError::Other("no artwork".into()))?;
    s.execute("artboard.setProps", &json!({"index": i, "x": b.x0, "y": b.y0, "width": b.width(), "height": b.height()}))
}

pub(super) fn artboard_fit_sel(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let st = s.doc()?;
    let b = st.doc.bounds_of(&st.selection.objects, true).ok_or_else(|| EngineError::Other("no selection bounds".into()))?;
    s.execute("artboard.setProps", &json!({"index": i, "x": b.x0, "y": b.y0, "width": b.width(), "height": b.height()}))
}
