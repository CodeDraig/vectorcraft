//! Glyph placement and alignment along text paths.

use super::{
    Affine, ArcPath, Ctx, Justify, LineInfo, Metrics, PathAlign, PathData, PathEffect, Range, SGlyph, TextObject, Vec2, is_rtl, para_bidi,
    style_metrics, visual_order,
};

/// The distance along type on a path each glyph takes: its advance measured `spacing` points above
/// the path (Type on a Path Options › Spacing), so glyphs close up round the outside of a curve and
/// open up round the inside; just the advances without spacing. Glyphs start `from` along the path.
fn path_steps(ap: &ArcPath, glyphs: &[SGlyph], from: f64, spacing: f64) -> Vec<f64> {
    let mut s = from;
    glyphs
        .iter()
        .map(|g| {
            let mut step = g.adv;
            if spacing != 0.0 && g.adv > 1e-9 {
                // How far the path turns across the glyph: positive bending away from its top.
                let ((_, a), (_, b)) = (ap.at(s), ap.at(s + g.adv));
                let curvature = a.cross(b).atan2(a.dot(b)) / g.adv;
                step = g.adv / (1.0 + curvature * spacing).clamp(0.25, 4.0);
            }
            s += step;
            step
        })
        .collect()
}

/// Lay type on a path out along `path` (text space), between its brackets.
pub(super) fn on_path(cx: &mut Ctx<'_>, paras: &[Range<usize>], t: &TextObject, path: &PathData) {
    cx.out.on_path = true;
    // Type on a path is one line: the first paragraph's attributes align it.
    let para = t.para_at(0);
    let mut sg = Vec::new();
    // The first paragraph's direction aligns the line (Auto) and sets the caret's.
    let mut rtl = None;
    for pr in paras {
        let text = cx.text;
        let bidi = para_bidi(text.get(pr.clone()).unwrap_or_default(), para.direction);
        rtl.get_or_insert(is_rtl(bidi.as_ref()) && !cx.vertical);
        let shaped = cx.shape_para(pr.clone(), bidi.as_ref());
        for i in visual_order(bidi.as_ref().filter(|_| !cx.vertical), pr.start, pr.clone(), &shaped) {
            if let Some(g) = shaped.get(i) {
                sg.push(g.clone());
            }
        }
    }
    let rtl = rtl.unwrap_or(false);
    let ap = ArcPath::new(path);
    let m = Metrics::max(&sg).map(|m| (m.asc, m.desc)).unwrap_or_else(|| {
        let s = style_metrics(cx.db, cx.style_at(0));
        (s.0, s.1)
    });
    let text_len = cx.text.len();
    if ap.is_empty() {
        cx.out.overflow = !sg.is_empty();
        cx.out.lines.push(LineInfo {
            rtl,
            baseline: 0.0,
            x0: 0.0,
            x1: 0.0,
            ascent: m.0,
            descent: m.1,
            start: 0,
            end: text_len,
            glyph_start: 0,
            glyph_end: 0,
            avail: (0.0, 0.0),
            region: 0,
        });
        return;
    }
    let centre = path.bounds().unwrap_or_default().center();
    let (from, to) = t.kind.path_span().unwrap_or((0.0, 1.0));
    let s_start = from * ap.len();
    let avail = (to - from) * ap.len();
    let spacing = if t.path_spacing.is_finite() { t.path_spacing } else { 0.0 };
    let mut steps = path_steps(&ap, &sg, s_start, spacing);
    let w: f64 = steps.iter().sum();
    let s0 = match para.justify {
        Justify::Auto if rtl => s_start + (avail - w).max(0.0),
        Justify::Center | Justify::JustifyCenter => s_start + ((avail - w) * 0.5).max(0.0),
        Justify::Right | Justify::JustifyRight => s_start + (avail - w).max(0.0),
        _ => s_start,
    };
    if spacing != 0.0 && s0 != s_start {
        // Spaced from where the alignment puts them, the curve under them is another.
        steps = path_steps(&ap, &sg, s0, spacing);
    }
    // Align to Path: how far down (glyph space) the type moves to run its ascender, centre or
    // descender along the path instead of its baseline.
    let rise = match t.path_align {
        PathAlign::Baseline => 0.0,
        PathAlign::Ascender => m.0,
        PathAlign::Descender => -m.1,
        PathAlign::Center => (m.0 - m.1) * 0.5,
    };
    let mut x = 0.0;
    for (g, &step) in sg.iter().zip(&steps) {
        let s = s0 + x;
        if s + step - s_start > avail + 1e-6 {
            cx.out.overflow = true;
            break;
        }
        let (p, dir) = ap.at(s + step * 0.5);
        let angle = dir.y.atan2(dir.x);
        let half = Affine::translate((-g.adv * 0.5, rise));
        // Glyph space: x along the advance, y down from the baseline; `pre` maps it onto the path.
        let pre = match t.path_effect {
            PathEffect::Rainbow => Affine::translate(p.to_vec2()) * Affine::rotate(angle) * half,
            // x axis along the tangent, y axis stays vertical.
            PathEffect::Skew => Affine::translate(p.to_vec2()) * Affine::new([dir.x, dir.y, 0.0, 1.0, 0.0, 0.0]) * half,
            // x axis stays horizontal (facing the path's direction), y axis perpendicular to the path.
            PathEffect::Ribbon3d => {
                let sx = if dir.x < 0.0 { -1.0 } else { 1.0 };
                Affine::translate(p.to_vec2()) * Affine::new([sx, 0.0, -dir.y * sx, dir.x * sx, 0.0, 0.0]) * half
            }
            PathEffect::StairStep => Affine::translate(ap.at(s).0.to_vec2()) * Affine::translate((0.0, rise)),
            // x axis along the tangent; vertical edges point at the path's centre (kept on the glyph's
            // up side, and never closer than ~17° to the baseline so glyphs stay legible).
            PathEffect::Gravity => {
                let n = Vec2::new(dir.y, -dir.x);
                let mut up = p - centre;
                up = if up.hypot() < 1e-9 { n } else { up / up.hypot() };
                if up.dot(n) < 0.0 {
                    up = -up;
                }
                if up.dot(n) < 0.3 {
                    let along = up - n * up.dot(n);
                    up = n * 0.3 + along / along.hypot().max(1e-9) * (1.0 - 0.09f64).sqrt();
                }
                Affine::translate(p.to_vec2()) * Affine::new([dir.x, dir.y, -up.x, -up.y, 0.0, 0.0]) * half
            }
        };
        // The caret's baseline moves with the type.
        let origin = p - dir * (g.adv * 0.5) + Vec2::new(-dir.y, dir.x) * rise;
        cx.emit(g, pre, origin, angle, g.adv, 0);
        x += step;
    }
    let (ps, _) = ap.at(s0);
    let (pe, _) = ap.at(s0 + x);
    cx.out.lines.push(LineInfo {
        rtl,
        baseline: ps.y,
        x0: ps.x,
        x1: pe.x,
        ascent: m.0,
        descent: m.1,
        start: 0,
        end: text_len,
        glyph_start: 0,
        glyph_end: cx.out.glyphs.len(),
        avail: (0.0, ap.len()),
        region: 0,
    });
}
