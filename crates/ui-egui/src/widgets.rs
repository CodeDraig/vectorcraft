//! Shared widgets in Illustrator's panel style. Colours come from theme tokens.

#[cfg(test)]
pub(crate) use fields::typed_unit;

mod fields;
pub use fields::{
    SpinPick, label_row, mixed_field, num_field, opt_field, plain_field, range_field, search_field, spin_field, spin_field_auto, spin_plain,
    text_field, text_presets,
};
pub(crate) use fields::{dialog_focus_flag, focus_field, round_to, set_bare_numbers_are_points, take_dialog_focus};
mod paint;
pub use paint::{ProxyClicks, chip_button, fill_stroke_proxy, gradient_chip, paint_chip, proxy_chip, swatch_tile};
mod popovers;
pub(crate) use popovers::combo;
pub use popovers::{dropdown, dropdown_names, dropdown_with, menu_scroll, popover, strip_scroll};
mod color;
pub(crate) use color::pointer_phase;
pub use color::{Live, WheelResponse, channel_slider, color_field, color_mesh, color_slider, harmony_wheel, hex_field};
mod panel_drag;
pub use panel_drag::{PanelDrag, SwatchRows, art_drop, doc_preview, drag_source};
mod transparency;
pub use transparency::{TransparencyEdit, blend_dropdown, blend_param, blend_param_dropdown, blend_separator_before, opacity_blend};

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::Unit;

use crate::theme::{self, Tokens};
use crate::{icons, scrub};

/// Square icon button; `selected` draws the pressed well.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str, selected: bool, size: f32) -> Response {
    let (_, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_icon_button(ui, resp, icon, tip, selected)
}

/// [`icon_button`] drawn for a response the caller made (its own id or sense).
pub fn paint_icon_button(ui: &mut Ui, resp: Response, icon: &str, tip: &str, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let rect = resp.rect;
    let size = rect.width();
    let bg = if selected {
        t.tool_active
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), bg);
    let pad = (size * 0.2).round();
    icons::paint(ui, icon, rect.shrink(pad), if selected { t.text } else { t.icon });
    if !tip.is_empty() { resp.on_hover_text(tl!(tip)) } else { resp }
}

/// Small flat text button (Quick Actions style).
pub fn flat_button(ui: &mut Ui, text: &str, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    flat_fill(ui, rect, &resp);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(1.0, if resp.hovered() { t.text } else { t.button_border }),
        StrokeKind::Inside,
    );
    ui.painter().with_clip_rect(rect).text(rect.center(), egui::Align2::CENTER_CENTER, tl!(text), egui::FontId::proportional(12.5), t.text_strong);
    resp
}

/// Width of a [`split_button`]'s arrow.
pub const SPLIT_ARROW: f32 = 18.0;

/// A [`flat_button`] `width` wide whose right end is a menu arrow (Image Trace ▾) → the
/// responses of the button and of its arrow.
pub fn split_button(ui: &mut Ui, text: &str, width: f32) -> (Response, Response) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    let split = rect.right() - SPLIT_ARROW;
    let main_rect = Rect::from_min_max(rect.min, pos2(split, rect.bottom()));
    let arrow_rect = Rect::from_min_max(pos2(split, rect.top()), rect.max);
    let main = ui.interact(main_rect, ui.id().with((text, "split")), Sense::click());
    let arrow = ui.interact(arrow_rect, ui.id().with((text, "split-arrow")), Sense::click());
    flat_fill(ui, main_rect, &main);
    flat_fill(ui, arrow_rect, &arrow);
    let border = if main.hovered() || arrow.hovered() { t.text } else { t.button_border };
    ui.painter().rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().vline(split, rect.y_range().shrink(4.0), Stroke::new(1.0, t.button_border));
    ui.painter().with_clip_rect(main_rect).text(
        main_rect.center(),
        egui::Align2::CENTER_CENTER,
        tl!(text),
        egui::FontId::proportional(12.5),
        t.text_strong,
    );
    icons::paint(ui, "chevron-down", Rect::from_center_size(arrow_rect.center(), vec2(12.0, 12.0)), t.text_dim);
    (main, arrow)
}

/// The pressed or hovered fill of a flat button part.
fn flat_fill(ui: &Ui, rect: Rect, resp: &Response) {
    let t = Tokens::get(ui.ctx());
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, CornerRadius::same(2), t.tool_active);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(2), t.hover);
    }
}

/// Blue call-to-action pill (dialog OK/Create).
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), theme::semibold(12.5), Color32::WHITE);
    let w = galley.size().x + 32.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(72.0), 28.0), Sense::click());
    let bg = if resp.hovered() { t.accent } else { t.accent_strong };
    ui.painter().rect_filled(rect, CornerRadius::same(14), bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    resp
}

/// Outlined secondary pill (dialog Cancel).
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), theme::semibold(12.5), t.text);
    let w = galley.size().x + 32.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(72.0), 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(14), t.hover);
    }
    ui.painter().rect_stroke(rect, CornerRadius::same(14), Stroke::new(1.5, t.text_dim), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
    resp
}

/// Bold section header (Properties panel).
pub fn section_header(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(2.0);
    ui.label(egui::RichText::new(tl!(text)).size(13.0).color(t.text));
    ui.add_space(2.0);
}

/// A collapsible section's header across the panel: a chevron (down while `open`) and `text`;
/// a click anywhere on it toggles the section. Whether it was clicked.
pub fn section_toggle(ui: &mut Ui, text: &str, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.hover);
    }
    let chevron = Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(12.0, 12.0));
    icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, chevron, if resp.hovered() { t.text_strong } else { t.icon });
    ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, tl!(text), theme::semibold(12.5), t.text);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, tl!(text)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Full-width 1 px divider with vertical margins.
pub fn divider(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.divider);
    ui.add_space(6.0);
}

/// A label in the panels' text colour; before a numeric field it scrubs the field
/// ([`scrub`]).
pub fn dim_label(ui: &mut Ui, text: &str) -> Response {
    let resp = dim_name(ui, tl!(text));
    scrub::note_label(ui, resp.rect);
    resp
}

/// A field's label in other styles (`text` as given): before a numeric field it scrubs the field
/// ([`scrub`]).
pub fn field_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    let resp = ui.label(text);
    scrub::note_label(ui, resp.rect);
    resp
}

/// [`dim_label`] for a name that is user or file data (a swatch, style, artboard or font name):
/// shown as it is, never translated.
pub fn dim_name(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).color(t.text).size(12.5))
}

/// The 3×3 reference point locator: nine squares, the eight outer ones joined by a line around
/// the box (the centre one stands alone), the current one filled. Returns a new index when clicked.
pub fn reference_point(ui: &mut Ui, current: usize) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    // 5 pt squares with 4 pt gaps; the origin is snapped to whole points so the 1 pt lines through
    // the squares' centres stay crisp.
    let (sq, step) = (5.0, 9.0);
    let size = 2.0 * step + sq;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let o = rect.min.round();
    let centre = |i: usize| o + Vec2::new(sq / 2.0 + (i % 3) as f32 * step, sq / 2.0 + (i / 3) as f32 * step);
    let line = Stroke::new(1.0, t.text_dim);
    ui.painter().rect_stroke(Rect::from_two_pos(centre(0), centre(8)), 0.0, line, StrokeKind::Middle);
    let mut out = None;
    for i in 0..9 {
        let r = Rect::from_center_size(centre(i), Vec2::splat(sq));
        let resp = ui.interact(r.expand(2.0), ui.id().with(("refpt", i)), Sense::click());
        if resp.clicked() {
            out = Some(i);
        }
        if i == current {
            ui.painter().rect_filled(r, 0.0, t.text);
        } else {
            ui.painter().rect_filled(r, 0.0, t.panel);
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, if resp.hovered() { t.text } else { t.text_dim }), StrokeKind::Inside);
        }
    }
    out
}

/// A toggle icon (e.g. eye / lock columns) — returns clicked.
pub fn toggle_icon(ui: &mut Ui, on_icon: &str, on: bool, size: f32, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if on {
        icons::paint(ui, on_icon, rect.shrink(size * 0.2), if resp.hovered() { t.text } else { t.icon });
    } else if resp.hovered() {
        icons::paint(ui, on_icon, rect.shrink(size * 0.2), t.text_disabled);
    }
    resp.on_hover_text(tl!(tip)).clicked()
}

// ---------- panel widgets (Swatches, Color, Stroke, Gradient, Appearance, …) ----------

/// Icon button that can be disabled (greyed, no hover, never clicked).
pub fn icon_button_enabled(ui: &mut Ui, icon: &str, tip: &str, selected: bool, enabled: bool, size: f32) -> Response {
    if enabled {
        return icon_button(ui, icon, tip, selected, size);
    }
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let pad = (size * 0.2).round();
    icons::paint(ui, icon, rect.shrink(pad), t.text_disabled);
    if tip.is_empty() { resp } else { resp.on_hover_text(tl!(tip)) }
}

/// Regular-weight panel sub-header ("Shape Modes:", "Align Objects:").
pub fn subheader(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(text)).size(12.5).color(t.text));
}

/// A label drawn with a dotted underline (Illustrator's link labels: "Stroke:", "Opacity:").
pub fn link_label(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), egui::FontId::proportional(12.5), t.text_strong);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(0.0, 3.0), Sense::click());
    let y = rect.top() + galley.size().y + 1.0;
    ui.painter().galley(rect.min, galley, t.text_strong);
    let mut x = rect.left();
    while x < rect.right() - 1.0 {
        ui.painter().line_segment([pos2(x, y), pos2((x + 1.0).min(rect.right()), y)], Stroke::new(1.0, t.text_dim));
        x += 2.5;
    }
    resp
}

/// The row of a checkbox or radio button: allocates a 13 pt box plus `label`, draws the label and
/// returns the box, the response and the box's border colour.
fn choice_row(ui: &mut Ui, label: &str, enabled: bool) -> (Rect, Response, Color32) {
    let t = Tokens::get(ui.ctx());
    let galley =
        ui.painter().layout_no_wrap(tl!(label).to_string(), egui::FontId::proportional(12.5), if enabled { t.text } else { t.text_disabled });
    let (rect, resp) =
        ui.allocate_exact_size(vec2(18.0 + galley.size().x, 20.0f32.max(galley.size().y)), if enabled { Sense::click() } else { Sense::hover() });
    let bx = Rect::from_min_size(pos2(rect.left(), rect.center().y - 6.5), Vec2::splat(13.0));
    ui.painter().galley(pos2(bx.right() + 5.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
    let border = if !enabled {
        t.divider
    } else if resp.hovered() {
        t.text
    } else {
        t.button_border
    };
    (bx, resp, border)
}

/// Panel-style checkbox with a disabled state. Returns true when toggled.
pub fn check(ui: &mut Ui, label: &str, value: bool, enabled: bool) -> bool {
    check3(ui, label, Some(value), enabled)
}

/// [`check`] with tooltip `tip` on the box and its label. Returns true when toggled.
pub fn check_tip(ui: &mut Ui, label: &str, value: bool, enabled: bool, tip: &str) -> bool {
    let (clicked, resp) = check_row(ui, label, Some(value), enabled);
    resp.on_hover_text(tip);
    clicked
}

/// Three-state [`check`]: `None` shows a dash (a neutral or mixed state). Returns true when clicked.
pub fn check3(ui: &mut Ui, label: &str, value: Option<bool>, enabled: bool) -> bool {
    check_row(ui, label, value, enabled).0
}

/// [`check3`], also returning the row's response (what a tooltip goes on: a widget around it never
/// counts as hovered under the box).
fn check_row(ui: &mut Ui, label: &str, value: Option<bool>, enabled: bool) -> (bool, Response) {
    let t = Tokens::get(ui.ctx());
    let (bx, resp, border) = choice_row(ui, label, enabled);
    let checked = value == Some(true);
    ui.painter().rect_filled(bx, CornerRadius::same(2), if checked && enabled { t.accent_strong } else { t.input });
    ui.painter().rect_stroke(bx, CornerRadius::same(2), Stroke::new(1.0, border), StrokeKind::Inside);
    match value {
        Some(true) => {
            let c = if enabled { Color32::WHITE } else { t.text_disabled };
            ui.painter().line_segment([bx.left_center() + vec2(3.0, 0.0), bx.center_bottom() + vec2(-1.0, -3.5)], Stroke::new(1.6, c));
            ui.painter().line_segment([bx.center_bottom() + vec2(-1.0, -3.5), bx.right_top() + vec2(-3.0, 3.0)], Stroke::new(1.6, c));
        }
        None => {
            let c = if enabled { t.text } else { t.text_disabled };
            ui.painter().line_segment([bx.left_center() + vec2(3.0, 0.0), bx.right_center() - vec2(3.0, 0.0)], Stroke::new(1.6, c));
        }
        Some(false) => {}
    }
    (enabled && resp.clicked(), resp)
}

/// Radio button in the style of [`check`], with a disabled state. Returns true when clicked.
pub fn radio(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (bx, resp, border) = choice_row(ui, label, enabled);
    let c = bx.center();
    ui.painter().circle(c, 6.0, if selected && enabled { t.accent_strong } else { t.input }, Stroke::new(1.0, border));
    if selected {
        ui.painter().circle_filled(c, 2.5, if enabled { Color32::WHITE } else { t.text_disabled });
    }
    enabled && resp.clicked()
}

/// A bordered list box (Swatches tiles, Appearance rows, Artboards list).
pub fn list_box<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.stroke(Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(1)).show(ui, add).inner
}

/// A bottom button bar separated from the content by a divider (panel footers).
pub fn bottom_bar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.divider);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        add(ui)
    });
}

/// A menu row for panel (≡) menus: label, optional check mark, disabled when not implemented.
pub fn menu_item(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    menu_item_name(ui, tl!(label), enabled, checked)
}

/// [`menu_item`] for a name that is user or file data (a library, a recent URL), never translated.
pub fn menu_item_name(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    let text = if checked { format!("✓ {label}") } else { format!("   {label}") };
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).size(12.5)).frame(false)).clicked()
}

/// A row of text tabs, the current one underlined in the accent colour (New Document's
/// categories, Document Setup's sections). Returns the clicked tab's index.
pub fn tab_bar(ui: &mut Ui, tabs: &[&str], current: usize) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (i, tab) in tabs.iter().enumerate() {
            let sel = i == current;
            let font = if sel { theme::semibold(13.0) } else { egui::FontId::proportional(13.0) };
            let galley = ui.painter().layout_no_wrap(tl!(tab).to_string(), font, t.text);
            let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x, 30.0), Sense::click());
            let color = if sel || resp.hovered() { t.text_strong } else { t.text_dim };
            ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0 - 2.0), galley, color);
            if sel {
                ui.painter().rect_filled(
                    Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.right_bottom()),
                    CornerRadius::same(1),
                    t.accent,
                );
            }
            if resp.clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// A page-orientation toggle drawn in code: a portrait or landscape sheet with a folded corner,
/// accent-filled when `selected`. Returns clicked.
pub fn orientation_button(ui: &mut Ui, landscape: bool, selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if resp.hovered() && !selected {
        ui.painter().rect_filled(rect, CornerRadius::same(3), t.hover);
    }
    let size = if landscape { vec2(16.0, 12.0) } else { vec2(12.0, 16.0) };
    let page = Rect::from_center_size(rect.center(), size);
    let color = if selected { t.accent } else { t.icon };
    let fold = 4.0;
    let outline =
        vec![page.left_top(), page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(0.0, fold), page.right_bottom(), page.left_bottom()];
    let fill = if selected { t.accent } else { Color32::TRANSPARENT };
    ui.painter().add(egui::Shape::convex_polygon(outline.clone(), fill, Stroke::new(1.2, color)));
    let corner = [page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(-fold, fold), page.right_top() + vec2(0.0, fold)];
    ui.painter().add(egui::Shape::line(corner.to_vec(), Stroke::new(1.2, if selected { t.panel } else { color })));
    resp.on_hover_text(tl!(tip)).clicked()
}

/// An artboard-layout toggle drawn in code (Rearrange All Artboards): small artboards in the grid
/// cells `tiles` (column, row), tinted, and over them their order as a line from a dot on the first
/// to an arrowhead on the last; accent-drawn in a pressed well when `selected`, greyed in a disabled
/// `ui`. `tip` is shown as it is (the caller translates it). Returns clicked.
pub fn layout_button(ui: &mut Ui, tiles: &[(u8, u8)], selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let enabled = ui.is_enabled();
    let bg = if selected {
        t.tool_active
    } else if resp.hovered() && enabled {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), bg);
    let color = if !enabled {
        t.text_disabled
    } else if selected {
        t.accent
    } else {
        t.icon
    };
    // Cells at most 10 points a side in a 20-point square: a row's artboards stand tall, a
    // column's lie flat.
    let (cols, rows) = tiles.iter().fold((1u8, 1u8), |(c, r), &(x, y)| (c.max(x.saturating_add(1)), r.max(y.saturating_add(1))));
    let cell = vec2((20.0 / f32::from(cols)).min(10.0), (20.0 / f32::from(rows)).min(10.0));
    let origin = rect.center() - vec2(f32::from(cols) * cell.x, f32::from(rows) * cell.y) / 2.0;
    let centre = |&(x, y): &(u8, u8)| origin + vec2((f32::from(x) + 0.5) * cell.x, (f32::from(y) + 0.5) * cell.y);
    for tile in tiles {
        ui.painter().rect_filled(Rect::from_center_size(centre(tile), cell - vec2(2.0, 2.0)), 1.0, color.gamma_multiply(0.35));
    }
    let path: Vec<Pos2> = tiles.iter().map(centre).collect();
    if let ([first, ..], [.., from, to]) = (path.as_slice(), path.as_slice()) {
        let (first, from, to) = (*first, *from, *to);
        let dir = (to - from).normalized();
        let side = dir.rot90() * 2.5;
        let head = vec![to + dir * 1.5, to - dir * 2.5 + side, to - dir * 2.5 - side];
        ui.painter().circle_filled(first, 1.6, color);
        ui.painter().add(egui::Shape::line(path, Stroke::new(1.2, color)));
        ui.painter().add(egui::Shape::convex_polygon(head, color, Stroke::NONE));
    }
    resp.on_hover_text(tip).clicked()
}

/// `region` of `doc` rendered on white, its longest side `px` pixels, as a texture named `name`
/// (artboard and page thumbnails).
pub fn region_texture(
    ctx: &egui::Context,
    renderer: &mut vectorcraft_render::Renderer,
    name: &str,
    doc: &vectorcraft_doc::Document,
    region: vectorcraft_geom::Rect,
    px: f64,
) -> egui::TextureHandle {
    let scale = px / region.width().max(region.height()).max(1.0);
    let img = renderer.render_region(doc, region, scale, true);
    let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}

/// The chevron cell on the right of a field that opens its preset list (rounded on the right).
fn chevron_cell(ui: &mut Ui, h: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (dr, dresp) = ui.allocate_exact_size(vec2(20.0, h), Sense::click());
    ui.painter().rect_filled(dr, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, t.input);
    ui.painter().rect_stroke(dr, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
    let c = if !ui.is_enabled() {
        t.text_disabled
    } else if dresp.hovered() {
        t.text_strong
    } else {
        t.icon
    };
    icons::paint(ui, "chevron-down", Rect::from_center_size(dr.center(), Vec2::splat(12.0)), c);
    dresp
}
