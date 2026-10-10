//! Paint chips, swatch tiles and the fill/stroke proxy.

use super::{Color32, Paint, PanelDrag, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, drag_source, icons, pos2, vec2};

/// Draw a paint preview (swatch chip) into `rect`.
pub fn paint_chip(ui: &Ui, rect: Rect, paint: &Paint) {
    let p = ui.painter();
    match paint {
        Paint::None => {
            p.rect_filled(rect, 0.0, Color32::WHITE);
            p.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.6, Color32::from_rgb(0xe0, 0x20, 0x20)));
        }
        Paint::Solid { color, .. } => {
            let [r, g, b, _] = color.to_rgba8(1.0);
            p.rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
        }
        Paint::Gradient(gp) => gradient_chip(ui, rect, &gp.gradient),
        Paint::Pattern { .. } => {
            p.rect_filled(rect, 0.0, Color32::from_gray(200));
            for i in 0..4 {
                let x = rect.left() + rect.width() * i as f32 / 4.0;
                p.line_segment([pos2(x, rect.bottom()), pos2(x + rect.width() / 4.0, rect.top())], Stroke::new(1.0, Color32::from_gray(90)));
            }
        }
    }
}

/// Draw a gradient preview into `rect` (linear left to right; radial from the centre).
pub fn gradient_chip(ui: &Ui, rect: Rect, gradient: &vectorcraft_color::Gradient) {
    let p = ui.painter();
    let radial = gradient.kind == vectorcraft_color::GradientKind::Radial;
    if radial {
        // Outer colour fills the corners beyond the largest circle.
        let (c, _) = gradient.sample(1.0);
        let [r, g, b, _] = c.to_rgba8(1.0);
        p.rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
    }
    let n = 24;
    let w = rect.width() / n as f32;
    for i in 0..n {
        let tt = (i as f32 + 0.5) / n as f32;
        if radial {
            let (c, _) = gradient.sample(1.0 - tt);
            let [r, g, b, _] = c.to_rgba8(1.0);
            let s = rect.width().min(rect.height()) * tt / 2.0;
            p.circle_filled(rect.center(), s.max(0.5), Color32::from_rgb(r, g, b));
        } else {
            let (c, _) = gradient.sample(tt);
            let [r, g, b, _] = c.to_rgba8(1.0);
            let rr = Rect::from_min_size(pos2(rect.left() + i as f32 * w, rect.top()), vec2(w + 0.5, rect.height()));
            p.rect_filled(rr, 0.0, Color32::from_rgb(r, g, b));
        }
    }
}

/// A Fill chip of `paint` into `rect` or, with `ring`, a Stroke chip: a frame `ring` wide round a
/// panel-coloured hole (returned; the whole chip for fills). A selection whose paints differ
/// (`mixed`) shows a white chip with a "?" (in the hole for strokes).
pub fn proxy_chip(ui: &Ui, rect: Rect, paint: &Paint, mixed: bool, ring: Option<f32>) -> Rect {
    let t = Tokens::get(ui.ctx());
    if mixed {
        ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
    } else {
        paint_chip(ui, rect, paint);
    }
    let (hole, mark) = match ring {
        Some(w) => {
            let inner = rect.shrink(w);
            ui.painter().rect_filled(inner, 0.0, t.panel);
            (inner, t.text_strong)
        }
        None => (rect, Color32::BLACK),
    };
    if mixed {
        ui.painter().text(hole.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(hole.height() * 0.8), mark);
    }
    hole
}

/// A chip that opens something (the Control bar's and Properties' Fill/Stroke chips, the Control
/// bar's Style picker): `draw` paints the `size`-square chip, which gets a frame, and with
/// `chevron` a chevron follows it.
pub fn chip_button(ui: &mut Ui, size: f32, chevron: bool, draw: impl FnOnce(&Ui, Rect)) -> Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(if chevron { size + 16.0 } else { size }, size), Sense::click());
    let chip = Rect::from_min_size(r.min, vec2(size, size)).shrink(2.0);
    draw(ui, chip);
    ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.input_border), StrokeKind::Outside);
    if chevron {
        icons::paint(ui, "chevron-down", Rect::from_center_size(pos2(chip.right() + 9.0, r.center().y), vec2(12.0, 12.0)), t.text_dim);
    }
    resp
}

/// What the user did on a [`fill_stroke_proxy`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProxyClicks {
    pub fill: bool,
    pub stroke: bool,
    pub swap: bool,
    pub default: bool,
    /// Double-clicked a square: open the Color Picker for it (`true` = the stroke).
    pub pick: Option<bool>,
}

/// The Fill/Stroke proxy pair (two overlapping squares). `mixed` (fill, stroke) draws a "?" square
/// for a selection whose objects differ.
pub fn fill_stroke_proxy(ui: &mut Ui, fill: &Paint, stroke: &Paint, mixed: (bool, bool), fill_active: bool, size: f32) -> ProxyClicks {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let s = size * 0.62;
    let fill_r = Rect::from_min_size(rect.min + vec2(0.0, 0.0), Vec2::splat(s));
    let stroke_r = Rect::from_min_size(rect.max - Vec2::splat(s), Vec2::splat(s));
    let stroke_resp = ui.interact(stroke_r, ui.id().with("stroke-proxy"), Sense::click_and_drag());
    let fill_resp = ui.interact(fill_r, ui.id().with("fill-proxy"), Sense::click_and_drag());
    // A proxy drags its paint (onto the Swatches panel or art), unless it shows "?".
    for (resp, paint, mixed) in [(&fill_resp, fill, mixed.0), (&stroke_resp, stroke, mixed.1)] {
        if !mixed {
            drag_source(ui, resp, || PanelDrag::paint(paint.clone()));
        }
    }
    let draw_fill = |ui: &Ui| {
        proxy_chip(ui, fill_r, fill, mixed.0, None);
        ui.painter().rect_stroke(fill_r, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Inside);
        ui.painter().rect_stroke(fill_r.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Outside);
    };
    let draw_stroke = |ui: &Ui| {
        let inner = proxy_chip(ui, stroke_r, stroke, mixed.1, Some(s * 0.28));
        ui.painter().rect_stroke(stroke_r, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Inside);
        ui.painter().rect_stroke(inner, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Outside);
        ui.painter().rect_stroke(stroke_r.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Outside);
    };
    if fill_active {
        draw_stroke(ui);
        draw_fill(ui);
    } else {
        draw_fill(ui);
        draw_stroke(ui);
    }
    // Swap arrow (top-right) and default (bottom-left) mini buttons.
    let swap_r = Rect::from_min_size(pos2(rect.right() - size * 0.3, rect.top()), Vec2::splat(size * 0.3));
    let def_r = Rect::from_min_size(pos2(rect.left(), rect.bottom() - size * 0.3), Vec2::splat(size * 0.3));
    let swap = ui.interact(swap_r, ui.id().with("swap-proxy"), Sense::click());
    let def = ui.interact(def_r, ui.id().with("default-proxy"), Sense::click());
    icons::paint(ui, "dc-swap", swap_r.shrink(1.0), if swap.hovered() { t.text } else { t.icon });
    let a = Rect::from_min_size(def_r.min + vec2(1.0, 1.0), Vec2::splat(size * 0.14));
    let b = Rect::from_min_size(def_r.min + vec2(size * 0.1, size * 0.1), Vec2::splat(size * 0.14));
    ui.painter().rect_filled(b, 0.0, Color32::BLACK);
    ui.painter().rect_stroke(b, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    ui.painter().rect_filled(a, 0.0, Color32::WHITE);
    ui.painter().rect_stroke(a, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
    let pick = if fill_resp.double_clicked() {
        Some(false)
    } else if stroke_resp.double_clicked() {
        Some(true)
    } else {
        None
    };
    ProxyClicks {
        fill: fill_resp.on_hover_text("Fill (X), double-click for the Color Picker").clicked(),
        stroke: stroke_resp.on_hover_text("Stroke (X), double-click for the Color Picker").clicked(),
        swap: swap.on_hover_text("Swap Fill and Stroke (Shift+X)").clicked(),
        default: def.on_hover_text("Default Fill and Stroke (D)").clicked(),
        pick,
    }
}

/// One swatch tile (Illustrator: 1 px dark frame, white inset on hover/selection).
pub fn swatch_tile(ui: &Ui, rect: Rect, paint: &Paint, selected: bool, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    paint_chip(ui, rect, paint);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    if selected || hovered {
        ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, if selected { t.accent } else { t.text }), StrokeKind::Outside);
    }
}
