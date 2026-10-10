//! Interactive color channels, hex entry and harmony wheels.

use super::{Color, Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, pos2, vec2};

/// Drag phase of a live slider/handle: previews while dragging, commits on release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Live {
    Idle,
    Dragging,
    Released,
}

/// A colour slider with a gradient track (`track(t)` gives the colour at 0..1) and a triangular
/// thumb below it, like Illustrator's Color panel. Returns the new normalized value and the phase.
pub fn color_slider(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let bar = Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 4.0), vec2(width - 8.0, 7.0));
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    gradient_strip(ui, bar, (bar.width() / 2.0).ceil().max(2.0) as usize, false, track);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    let v = value.clamp(0.0, 1.0);
    let x = bar.left() + v * bar.width();
    let tip = pos2(x, bar.bottom() - 1.0);
    let thumb = vec![tip, pos2(x + 5.5, tip.y + 6.0), pos2(x + 5.5, tip.y + 10.0), pos2(x - 5.5, tip.y + 10.0), pos2(x - 5.5, tip.y + 6.0)];
    let fill = if resp.dragged() || resp.hovered() { Color32::WHITE } else { t.icon };
    ui.painter().add(egui::Shape::convex_polygon(thumb, fill, Stroke::new(1.0, t.border)));
    match pointer_phase(&resp) {
        Some((p, phase)) => (Some(((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0)), phase),
        None => (None, Live::Idle),
    }
}

/// The pointer of a click or drag on `resp` and its phase (dragging, or released on click/drop).
pub(crate) fn pointer_phase(resp: &Response) -> Option<(Pos2, Live)> {
    if !(resp.dragged() || resp.clicked() || resp.drag_stopped()) {
        return None;
    }
    let p = resp.interact_pointer_pos()?;
    Some((p, if resp.dragged() && !resp.drag_stopped() { Live::Dragging } else { Live::Released }))
}

/// A gradient strip into `rect`: `n` bands along x (or up y when `vertical`), each filled with
/// `track(t)` at its centre.
fn gradient_strip(ui: &Ui, rect: Rect, n: usize, vertical: bool, track: &dyn Fn(f32) -> Color32) {
    let len = if vertical { rect.height() } else { rect.width() };
    let seg = len / n as f32;
    for i in 0..n {
        let r = if vertical {
            Rect::from_min_size(pos2(rect.left(), rect.bottom() - (i + 1) as f32 * seg), vec2(rect.width(), seg + 0.6))
        } else {
            Rect::from_min_size(pos2(rect.left() + i as f32 * seg, rect.top()), vec2(seg + 0.6, rect.height()))
        };
        ui.painter().rect_filled(r, 0.0, track((i as f32 + 0.5) / n as f32));
    }
}

/// The Color Picker's 2D colour field: `color_at(x, y)` gives the colour at a point (0..1 each, y
/// down) and a ring marks `pos`. Returns the clicked or dragged point and the phase.
pub fn color_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    size: Vec2,
    pos: (f32, f32),
    color_at: &dyn Fn(f32, f32) -> Color32,
) -> (Option<(f32, f32)>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(6.0));
    painter.add(color_mesh(rect, (32, 32), color_at));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    let c = pos2(rect.left() + pos.0.clamp(0.0, 1.0) * rect.width(), rect.top() + pos.1.clamp(0.0, 1.0) * rect.height());
    painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::BLACK));
    painter.circle_stroke(c, 4.0, Stroke::new(1.0, Color32::WHITE));
    match pointer_phase(&resp) {
        Some((p, phase)) => {
            let f = |v: f32, lo: f32, len: f32| ((v - lo) / len).clamp(0.0, 1.0);
            (Some((f(p.x, rect.left(), rect.width()), f(p.y, rect.top(), rect.height()))), phase)
        }
        None => (None, Live::Idle),
    }
}

/// A smoothly shaded colour area: a `cols` × `rows` grid over `rect` with `color_at(x, y)` (0..1
/// each, y down) at its vertices.
pub fn color_mesh(rect: Rect, (cols, rows): (u32, u32), color_at: &dyn Fn(f32, f32) -> Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    let row = cols + 1;
    mesh.reserve_vertices((row * (rows + 1)) as usize);
    mesh.reserve_triangles((cols * rows * 2) as usize);
    for j in 0..=rows {
        for i in 0..=cols {
            let (x, y) = (i as f32 / cols as f32, j as f32 / rows as f32);
            mesh.colored_vertex(pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()), color_at(x, y));
        }
    }
    for j in 0..rows {
        for i in 0..cols {
            let a = j * row + i;
            mesh.add_triangle(a, a + 1, a + row);
            mesh.add_triangle(a + 1, a + row + 1, a + row);
        }
    }
    egui::Shape::mesh(mesh)
}

/// The Color Picker's vertical channel slider: `track(t)` gives the colour at 0..1 (bottom to top)
/// and arrows on both sides mark `value`. Returns the new value and the phase.
pub fn channel_slider(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f32,
    size: Vec2,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let bar = rect.shrink2(vec2(6.0, 0.0));
    gradient_strip(ui, bar, (bar.height() / 2.0).ceil().max(2.0) as usize, true, track);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    let y = bar.bottom() - value.clamp(0.0, 1.0) * bar.height();
    let fill = if resp.dragged() || resp.hovered() { t.text_strong } else { t.icon };
    for (x, d) in [(rect.left(), 1.0), (rect.right(), -1.0)] {
        let tip = pos2(x + d * 5.0, y);
        ui.painter().add(egui::Shape::convex_polygon(vec![tip, pos2(x, y + 4.0 * d), pos2(x, y - 4.0 * d)], fill, Stroke::NONE));
    }
    match pointer_phase(&resp) {
        Some((p, phase)) => (Some(((bar.bottom() - p.y) / bar.height()).clamp(0.0, 1.0)), phase),
        None => (None, Live::Idle),
    }
}

/// A hex colour field (`E67828`, no `#`) after a `#` label. Returns the text on commit when it
/// changed.
pub fn hex_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, hex: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new("#").size(15.0).color(t.text));
    let id = ui.id().with(id);
    let editing = ui.memory(|m| m.has_focus(id));
    let mut buf: String = if editing { ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| hex.to_string()) } else { hex.to_string() };
    let resp = egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, if editing { t.accent } else { t.input_border }))
        .corner_radius(2)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut buf)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_width(62.0)
                    .char_limit(7)
                    .font(egui::FontId::proportional(12.5))
                    .text_color(t.text_strong),
            )
        })
        .inner;
    let commit = resp.lost_focus() && buf != hex;
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    commit.then_some(buf)
}

/// Position of hue `h` (degrees) and saturation `s` on a wheel of radius `r` around `c`.
fn wheel_pos(c: Pos2, r: f32, h: f32, s: f32) -> Pos2 {
    let a = h.to_radians();
    c + vec2(a.cos(), -a.sin()) * r * s
}

/// The hue and saturation wheel at brightness `v`: hue around, saturation outwards.
fn wheel_shape(c: Pos2, r: f32, v: f32) -> egui::Shape {
    const SEG: u32 = 72;
    const RINGS: u32 = 8;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, crate::panels::c32(&Color::from_hsb(0.0, 0.0, v)));
    for ring in 1..=RINGS {
        let s = ring as f32 / RINGS as f32;
        for k in 0..SEG {
            let h = k as f32 * 360.0 / SEG as f32;
            mesh.colored_vertex(wheel_pos(c, r, h, s), crate::panels::c32(&Color::from_hsb(h, s, v)));
        }
    }
    let at = |ring: u32, k: u32| if ring == 0 { 0 } else { 1 + (ring - 1) * SEG + k % SEG };
    for ring in 0..RINGS {
        for k in 0..SEG {
            if ring == 0 {
                mesh.add_triangle(0, at(1, k), at(1, k + 1));
            } else {
                mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
                mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
            }
        }
    }
    egui::Shape::mesh(mesh)
}

/// What a [`harmony_wheel`] did this frame.
#[derive(Default)]
pub struct WheelResponse {
    /// The colour whose marker was pressed (it becomes the selected one).
    pub pressed: Option<usize>,
    /// A colour moved to a new hue, saturation and brightness (`[h°, s, v]`), by dragging its
    /// marker or (the selected colour) the brightness slider.
    pub moved: Option<(usize, [f32; 3])>,
}

/// The harmony wheel of Recolor Artwork's Edit tab and the Color Themes panel: a hue and saturation
/// wheel `size` across (hue around, saturation outwards) at the selected colour's brightness, a
/// marker per colour joined to the centre (the selected colour, `base`, larger) and a Brightness
/// slider for the selected colour below it.
pub fn harmony_wheel(ui: &mut Ui, id: &str, size: f32, colors: &[Color], base: Option<usize>) -> WheelResponse {
    let t = Tokens::get(ui.ctx());
    let mut out = WheelResponse::default();
    let base_hsb = base.and_then(|b| colors.get(b)).map(Color::to_hsb);
    let v = base_hsb.map_or(1.0, |c| c[2]);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - size) / 2.0).max(0.0));
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
        let (c, r) = (rect.center(), size / 2.0 - 10.0);
        let painter = ui.painter_at(rect);
        painter.add(wheel_shape(c, r, v.max(0.15)));
        painter.circle_stroke(c, r, Stroke::new(1.0, t.border));
        let at = |col: &Color| {
            let [h, s, _] = col.to_hsb();
            wheel_pos(c, r, h, s)
        };
        for (i, col) in colors.iter().enumerate() {
            let (p, big) = (at(col), Some(i) == base);
            painter.line_segment([c, p], Stroke::new(1.0, Color32::from_white_alpha(160)));
            painter.circle(p, if big { 8.0 } else { 5.5 }, crate::panels::c32(col), Stroke::new(1.5, Color32::WHITE));
            painter.circle_stroke(p, if big { 9.0 } else { 6.5 }, Stroke::new(1.0, Color32::BLACK));
        }
        // Drag a marker: the nearest one within reach of the press.
        let drag = ui.id().with((id, "drag"));
        if (resp.drag_started() || resp.clicked())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let near = colors.iter().map(at).enumerate().map(|(i, m)| (i, m.distance(p))).filter(|x| x.1 <= 14.0).min_by(|a, b| a.1.total_cmp(&b.1));
            out.pressed = near.map(|x| x.0);
            ui.data_mut(|m| m.insert_temp(drag, out.pressed));
        }
        if resp.dragged()
            && let (Some(i), Some(p)) = (ui.data(|m| m.get_temp::<Option<usize>>(drag)).flatten(), resp.interact_pointer_pos())
            && let Some(old) = colors.get(i)
        {
            let dv = p - c;
            let h = (-dv.y).atan2(dv.x).to_degrees().rem_euclid(360.0);
            out.moved = Some((i, [h, (dv.length() / r).clamp(0.0, 1.0), old.to_hsb()[2]]));
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - size - 76.0) / 2.0).max(0.0));
        ui.add_sized([70.0, 22.0], egui::Label::new(egui::RichText::new("Brightness").color(t.text_dim)));
        if let (Some(b), Some([h, s, _])) = (base, base_hsb)
            && let (Some(nv), _) = color_slider(ui, (id, "brightness"), v, size, &|x| crate::panels::c32(&Color::from_hsb(h, s, x)))
        {
            out.moved = Some((b, [h, s, nv]));
        }
    });
    out
}
