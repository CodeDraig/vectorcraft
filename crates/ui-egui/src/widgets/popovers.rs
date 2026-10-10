//! Dropdowns, searchable choices and bounded popup scrolling.

use super::{CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, vec2};

/// A compact dropdown of interface labels (shown in the UI language). Returns the chosen index.
pub fn dropdown(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[&str], width: f32) -> Option<usize> {
    dropdown_with(ui, id, current, options, width, |_| true)
}

/// [`dropdown`] of names that are user, file or system data (font styles, artboards, presets the
/// user saved, printers, profiles), shown as they are, never translated. For a list that mixes
/// built-in labels with such names, translate the built-in ones (`tl!`) before passing them.
pub fn dropdown_names(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[&str], width: f32) -> Option<usize> {
    combo(ui, id, current, width, false, |ui| {
        let mut chosen = None;
        for (i, o) in options.iter().enumerate() {
            if ui.add(egui::Button::selectable(*o == current, *o)).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// [`dropdown`] whose options can be disabled (`enabled(index)`: greyed and not choosable).
pub fn dropdown_with(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    options: &[&str],
    width: f32,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    combo(ui, id, tl!(current), width, false, |ui| {
        let mut chosen = None;
        for (i, o) in options.iter().enumerate() {
            if ui.add_enabled(enabled(i), egui::Button::selectable(*o == current, tl!(o))).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// The recessed combo box of [`dropdown`] showing `current`; `list` draws the options and returns
/// the chosen one. A `searchable` list (a search field over a list it scrolls itself) stays open
/// while it is clicked: it closes itself when an option is chosen.
pub(crate) fn combo<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    width: f32,
    searchable: bool,
    list: impl FnOnce(&mut Ui) -> Option<R>,
) -> Option<R> {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(CornerRadius::same(3))
        .show(ui, |ui| {
            if searchable {
                return searchable_combo(ui, ui.id().with(id), current, width - 4.0, list);
            }
            egui::ComboBox::from_id_salt(ui.id().with(id))
                .selected_text(egui::RichText::new(current).size(12.0))
                .width(width - 4.0)
                .height(ui.spacing().combo_height)
                .show_ui(ui, list)
                .inner
                .flatten()
        })
        .inner
}

/// A dropdown whose list keeps its own search field and scrolling rows (the font menu): drawn as
/// egui's combo box, but in a popup without the scroll area egui puts around a combo's list, so
/// the field stays at the top and only the rows scroll (#555). A click outside closes it.
fn searchable_combo<R>(ui: &mut Ui, id: egui::Id, current: &str, width: f32, list: impl FnOnce(&mut Ui) -> Option<R>) -> Option<R> {
    let popup = id.with("popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let pad = ui.spacing().button_padding;
    let icon = Vec2::splat(ui.spacing().icon_width);
    let galley = egui::WidgetText::from(egui::RichText::new(current).size(12.0)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width - 2.0 * pad.x - ui.spacing().icon_spacing - icon.x,
        egui::TextStyle::Button,
    );
    let height = (galley.size().y.max(icon.y) + 2.0 * pad.y).max(ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let v = if open { ui.visuals().widgets.open } else { *ui.style().interact(&resp) };
        ui.painter().rect(rect.expand(v.expansion), v.corner_radius, v.weak_bg_fill, v.bg_stroke, StrokeKind::Inside);
        let inner = rect.shrink2(pad);
        ui.painter().galley(egui::Align2::LEFT_CENTER.align_size_within_rect(galley.size(), inner).min, galley, v.text_color());
        // egui's combo arrow: a downward triangle.
        let arrow =
            Rect::from_center_size(egui::Align2::RIGHT_CENTER.align_size_within_rect(icon, inner).center(), vec2(icon.x * 0.7, icon.y * 0.45));
        ui.painter().add(egui::Shape::convex_polygon(
            vec![arrow.left_top(), arrow.right_top(), arrow.center_bottom()],
            v.fg_stroke.color,
            Stroke::NONE,
        ));
    }
    // It opens below the button, or above it when there is more room there, and the list gets
    // the room left (the popup's frame aside).
    const GAP: f32 = 12.0;
    let screen = ui.ctx().content_rect();
    let frame = ui.spacing().menu_margin.sum().y + GAP;
    let (below, above) = (screen.bottom() - rect.bottom() - frame, rect.top() - screen.top() - frame);
    let (align, room) = if below >= above { (egui::RectAlign::BOTTOM_START, below) } else { (egui::RectAlign::TOP_START, above) };
    egui::Popup::menu(&resp)
        .id(popup)
        .width(rect.width())
        .align(align)
        .align_alternatives(&[])
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(ui.available_width());
            ui.set_max_height(room);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            list(ui)
        })
        .and_then(|r| r.inner)
}

/// A popover anchored to `resp`, opened and closed by `toggle` (usually `resp.clicked()`), and
/// closed by Escape, a click outside it and outside any popup it opened, or a frame in which its
/// anchor isn't shown. Use it instead of `egui::Popup::menu` when the popover holds dropdowns or
/// popups of its own: egui remembers one open popup at a time, so opening a dropdown inside a
/// remembered popup would close the popup (and the dropdown with it). This one keeps its open
/// state itself: the last frame it was open in.
pub fn popover<R>(resp: &Response, toggle: bool, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let ctx = &resp.ctx;
    let id = resp.id.with("popover");
    let frame = ctx.cumulative_frame_nr();
    let was_open = ctx.data(|d| d.get_temp::<u64>(id)).is_some_and(|f| f + 1 >= frame);
    let mut open = if toggle {
        !was_open
    } else {
        // Popups (this one and those it opened) are on foreground layers; a click anywhere
        // else closes it.
        let clicked_elsewhere = ctx
            .input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()).flatten())
            .is_some_and(|pos| ctx.layer_id_at(pos).is_none_or(|l| l.order != egui::Order::Foreground));
        was_open && !clicked_elsewhere
    };
    let inner =
        egui::Popup::menu(resp).id(id).open_bool(&mut open).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(content).map(|r| r.inner);
    ctx.data_mut(|d| {
        if open {
            d.insert_temp(id, frame);
        } else {
            d.remove::<u64>(id);
        }
    });
    inner
}

/// The body of a menu or popup list: as tall as its items up to the bottom of the window, and
/// scrolling only past that, so long menus (Window, Effect, a panel's menu) stay reachable on
/// small windows.
pub fn menu_scroll<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    // Room left below the popup's first item, less the popup frame and a small gap.
    const BOTTOM_GAP: f32 = 12.0;
    const MIN_HEIGHT: f32 = 120.0;
    let room = (ui.ctx().content_rect().bottom() - ui.next_widget_position().y - BOTTOM_GAP).max(MIN_HEIGHT);
    // The minimum lets the list grow past a popup's default 400 pt size; it still shrinks to
    // its items when they need less.
    egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room).show(ui, add_contents).inner
}

/// A column of tool or panel buttons (the toolbar, the dock's icon column) that scrolls when the
/// window is too short for it: with the mouse wheel, or the scroll bar shown while the pointer is
/// over it.
/// Its scroll bar stays a slim line at the edge, even while scrolled or dragged, so it never covers
/// the buttons (#808).
pub fn strip_scroll<R>(ui: &mut Ui, id: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().scroll = egui::style::ScrollStyle {
            bar_width: 4.0,
            floating_width: 2.0,
            bar_inner_margin: 0.0,
            bar_outer_margin: 1.0,
            ..egui::style::ScrollStyle::floating()
        };
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, true]).show(ui, add_contents).inner
    })
    .inner
}
