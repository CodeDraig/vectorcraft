//! Numeric and text entry, unit parsing, scrubbing and focus.

use super::{CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Unit, chevron_cell, pos2, scrub, vec2};

/// Preferences › Units › Numbers Without Units Are Points (#394; on until set): the app sets it
/// when the preference changes (`prefs_dialog::apply_runtime`), the length fields read it.
pub(crate) fn set_bare_numbers_are_points(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("bare_numbers_are_points"), on));
}

/// The unit a number typed with no unit into a field in `unit` is read in: `unit`, or for a field
/// in picas points with Numbers Without Units Are Points on (where `12` could be either). A typed
/// unit (`12 mm`, `2p6`, `1in`) always wins.
pub(crate) fn typed_unit(ctx: &egui::Context, unit: Unit) -> Unit {
    let bare_points = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("bare_numbers_are_points"))).unwrap_or(true);
    if bare_points && unit == Unit::Picas { Unit::Points } else { unit }
}

/// A recessed numeric field showing `value` (points) in `unit`. Returns the new value (points)
/// when the user commits (Enter / focus loss). Supports unit suffixes and arithmetic; a number
/// without a unit is in `unit` ([`typed_unit`]: in points in a picas field with Numbers Without
/// Units Are Points on).
pub fn num_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<f64>, unit: Unit, width: f32) -> Option<f64> {
    let t = Tokens::get(ui.ctx());
    let id = ui.id().with(id);
    let shown = value.map(|v| unit.format(v)).unwrap_or_default();
    let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.clone());
    let editing = ui.memory(|m| m.has_focus(id));
    if !editing {
        buf = shown.clone();
    }
    take_dialog_focus(ui, id, &buf);
    let (rect, resp) = ui
        .allocate_ui_with_layout(vec2(width, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(width);
            let framed = egui::Frame::NONE
                .fill(t.input)
                .stroke(Stroke::new(1.0, if editing { t.accent } else { t.input_border }))
                .corner_radius(CornerRadius::same(2))
                .inner_margin(egui::Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut buf)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .desired_width(width - 14.0)
                            .min_size(vec2(width - 14.0, 0.0))
                            .font(egui::FontId::proportional(12.5))
                            .text_color(t.text_strong),
                    )
                });
            (framed.response.rect, framed.inner)
        })
        .inner;
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step in the field's unit; a drag on its label scrubs it.
    let read = typed_unit(ui.ctx(), unit);
    let stepped = step_value(ui, &resp, &mut buf, |b| Some(unit.from_pt(read.parse(b)?)), |v| unit.format(unit.to_pt(v)));
    let scrubbed = scrub::field(ui, id, rect, &buf, value.map(|v| unit.from_pt(v)), STEP_DECIMALS);
    let commit = resp.lost_focus() && buf != shown;
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    if commit { read.parse(&buf) } else { stepped.or(scrubbed).map(|v| unit.to_pt(v)) }
}

/// The places a stepped value in a unit field keeps (no 12.300000001).
const STEP_DECIMALS: i32 = 3;

/// `v` rounded to `decimals` places.
pub(crate) fn round_to(v: f64, decimals: i32) -> f64 {
    let scale = 10f64.powi(decimals.clamp(0, 6));
    (v * scale).round() / scale
}

/// One step of a numeric field with each modifier, most specific first (a plain pattern would also
/// match Shift): ten with Shift, a tenth with Ctrl/Cmd, else one.
const STEPS: [(egui::Modifiers, f64); 3] = [(egui::Modifiers::SHIFT, 10.0), (egui::Modifiers::COMMAND, 0.1), (egui::Modifiers::NONE, 1.0)];

/// ↑/↓ in focused numeric field `resp`, or the mouse wheel turned over it, step the number `buf`
/// shows (`read` parses it, `show` formats it) by one per press or wheel notch ([`STEPS`]: Shift
/// ten, Ctrl/Cmd a tenth), applied at once as in Illustrator's panels and dialogs. The new text
/// replaces `buf`, all selected so typing replaces it. Returns the new number when a step was
/// taken. While the pointer is over the focused field the wheel is the field's (the panel under it
/// doesn't scroll); an unfocused field leaves the wheel to the panel and the canvas.
fn step_value(ui: &Ui, resp: &Response, buf: &mut String, read: impl Fn(&str) -> Option<f64>, show: impl Fn(f64) -> String) -> Option<f64> {
    // Memory focus, as the field's highlight and its typing use: `Response::has_focus` also needs the
    // window to report keyboard focus.
    if !ui.memory(|m| m.has_focus(resp.id)) {
        return None;
    }
    use egui::{Event, Key, MouseWheelUnit};
    let wheel = resp.contains_pointer();
    // Wheel turns in notches: a line each, or `line` points of a trackpad or a fine wheel; what is
    // left of a notch carries over to the next turn.
    let (line, notch_id) = (ui.ctx().options(|o| o.input_options.line_scroll_speed), resp.id.with("wheel"));
    let mut notches: f32 = if wheel { ui.data(|d| d.get_temp(notch_id)).unwrap_or(0.0) } else { 0.0 };
    let mut steps = 0.0;
    ui.input_mut(|i| {
        for (mods, size) in STEPS {
            let up = i.count_and_consume_key(mods, Key::ArrowUp) as f64;
            let down = i.count_and_consume_key(mods, Key::ArrowDown) as f64;
            steps += size * (up - down);
        }
        if !wheel {
            return;
        }
        i.events.retain(|e| {
            let Event::MouseWheel { unit, delta, modifiers, .. } = e else { return true };
            let Some((_, size)) = STEPS.iter().find(|(m, _)| modifiers.matches_logically(*m)) else { return true };
            // Shift may turn the wheel sideways (the Mac does).
            let d = if modifiers.shift { delta.x + delta.y } else { delta.y };
            notches += match unit {
                MouseWheelUnit::Point => d / line.max(1.0),
                MouseWheelUnit::Line | MouseWheelUnit::Page => d,
            };
            let whole = notches.trunc();
            notches -= whole;
            steps += size * f64::from(whole);
            false
        });
        // The rest of a turn egui spreads over the next frames scrolls nothing either.
        i.smooth_scroll_delta = egui::Vec2::ZERO;
    });
    if wheel {
        ui.data_mut(|d| d.insert_temp(notch_id, notches));
    }
    if steps == 0.0 {
        return None;
    }
    // Rounded as fields show it.
    let v = round_to(read(buf)? + steps, STEP_DECIMALS);
    *buf = show(v);
    ui.data_mut(|d| d.insert_temp(resp.id, buf.clone()));
    select_all(ui, resp.id, buf);
    Some(v)
}

/// The flag `dialogs::show` raises while it draws the body of a dialog that has just opened; the
/// first field drawn takes it ([`take_dialog_focus`]).
pub(crate) fn dialog_focus_flag() -> egui::Id {
    egui::Id::new("dialog-focus-first-field")
}

/// If field `id` (showing `text`) is the first field of a dialog that has just opened, give it the
/// keyboard focus with all of `text` selected: typing replaces the value and Enter applies it.
pub(crate) fn take_dialog_focus(ui: &Ui, id: egui::Id, text: &str) {
    // A new window's first frame only measures its contents: nothing can take focus yet.
    if ui.is_sizing_pass() || ui.data_mut(|d| d.remove_temp::<bool>(dialog_focus_flag())).is_none() {
        return;
    }
    focus_field(ui, id, text);
}

/// Give text field `id` the keyboard focus with all of `text` selected.
pub(crate) fn focus_field(ui: &Ui, id: egui::Id, text: &str) {
    select_all(ui, id, text);
    ui.memory_mut(|m| m.request_focus(id));
}

/// Select all of `text` in text field `id`.
fn select_all(ui: &Ui, id: egui::Id, text: &str) {
    let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
    st.cursor.set_char_range(Some(all));
    st.store(ui.ctx(), id);
}

/// Numeric fields take the whole text, unit included, when they gain focus or are double-clicked
/// (egui would select one word): typing a number or an expression replaces it.
fn select_all_on_focus(ui: &Ui, resp: &Response, text: &str) {
    if !(resp.gained_focus() || resp.double_clicked()) {
        return;
    }
    if egui::TextEdit::load_state(ui.ctx(), resp.id).is_some() {
        select_all(ui, resp.id, text);
        ui.ctx().request_repaint();
    }
}

/// The recessed text box of the panel fields showing `shown`, `rows` lines tall (1: single line),
/// its text kept under `id` while it has focus. Returns the text (as edited), the text edit's
/// response and the box.
fn recessed_text(ui: &mut Ui, id: egui::Id, shown: &str, width: f32, rows: usize) -> (String, Response, Rect) {
    let t = Tokens::get(ui.ctx());
    let editing = ui.memory(|m| m.has_focus(id));
    let mut buf: String = if editing { ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.to_string()) } else { shown.to_string() };
    let height = 26.0 + 16.0 * (rows.max(1) - 1) as f32;
    let (rect, resp) = ui
        .allocate_ui_with_layout(vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(width);
            let framed = egui::Frame::NONE
                .fill(t.input)
                .stroke(Stroke::new(1.0, if editing { t.accent } else { t.input_border }))
                .corner_radius(CornerRadius::same(2))
                .inner_margin(egui::Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    let edit = if rows > 1 { egui::TextEdit::multiline(&mut buf).desired_rows(rows) } else { egui::TextEdit::singleline(&mut buf) };
                    ui.add(
                        edit.id(id)
                            .frame(egui::Frame::NONE)
                            .desired_width(width - 14.0)
                            .min_size(vec2(width - 14.0, 0.0))
                            .font(egui::FontId::proportional(12.5))
                            .text_color(t.text_strong),
                    )
                });
            (framed.response.rect, framed.inner)
        })
        .inner;
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    (buf, resp, rect)
}

/// A recessed text field showing `value` (blank when `None`: the selection's values differ),
/// `rows` lines tall (1: single line, which Enter commits). Returns the new text, trimmed, when a
/// change is committed (Enter or focus loss).
pub fn text_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<&str>, width: f32, rows: usize) -> Option<String> {
    let id = ui.id().with(id);
    let shown = value.unwrap_or_default();
    let (buf, resp, _) = recessed_text(ui, id, shown, width, rows);
    (resp.lost_focus() && buf.trim() != shown).then(|| buf.trim().to_string())
}

/// A form row: `label` in a column `label_width` wide, then what `add` draws.
pub fn label_row(ui: &mut Ui, label: &str, label_width: f32, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(label_width, 24.0), Sense::hover());
        let t = Tokens::get(ui.ctx());
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, tl!(label), egui::FontId::proportional(12.5), t.text);
        scrub::note_label(ui, r);
        add(ui);
    });
}

/// A plain number field (percent, degrees, counts) with optional suffix.
pub fn plain_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: f64, suffix: &str, decimals: usize, width: f32) -> Option<f64> {
    mixed_field(ui, id, Some(value), suffix, decimals, width)
}

/// [`plain_field`] for a number kept within `range` (the values of dialogs and preferences): what is
/// typed, stepped or scrubbed is clamped into it and rounded to `decimals` places (0: a count).
pub fn range_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    let (lo, hi) = (*range.start(), *range.end());
    let places = decimals.min(6) as i32;
    plain_field(ui, id, value.clamp(lo, hi), suffix, decimals, width).map(|v| round_to(v, places).clamp(lo, hi))
}

/// [`plain_field`] for a value the selected objects may not share: `None` shows a blank field.
pub fn mixed_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: Option<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    let id = ui.id().with(id);
    let show = |value: f64| {
        let s = format!("{:.*}", decimals, value);
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        format!("{s}{suffix}")
    };
    // The suffix (and %, °) may follow any operand: `45*2°`, `50% / 2`.
    let read = |buf: &str| {
        let suffix = suffix.trim();
        let bare = if suffix.is_empty() { buf.to_string() } else { buf.replace(suffix, "") };
        vectorcraft_doc::parse_number(&bare.replace(['%', '°'], ""))
    };
    let shown = value.map(show).unwrap_or_default();
    let (mut buf, resp, rect) = recessed_text(ui, id, &shown, width, 1);
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step at the field's precision (a count ignores Ctrl/Cmd's tenth); a drag on
    // its label scrubs it.
    let decimals = decimals.min(6) as i32;
    let stepped = step_value(ui, &resp, &mut buf, read, show).map(|v| round_to(v, decimals));
    let scrubbed = scrub::field(ui, id, rect, &buf, value, decimals);
    if resp.lost_focus() && buf != shown { read(&buf) } else { stepped.or(scrubbed).filter(|&v| Some(v) != value) }
}

/// Numeric field with an up/down spinner on the left and a preset dropdown on the right
/// (Stroke weight, font size…): an editable combo box whose field keeps typing, ↑/↓ stepping and
/// label scrubbing. `value` (points) shows in `unit`, blank when `None` (the selection's values
/// differ); `presets` (points) are listed in `unit` too, and empty = no dropdown. Returns the
/// committed, stepped or picked value.
#[allow(clippy::too_many_arguments)]
pub fn spin_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    value: Option<f64>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    spin_field_auto(ui, id, value, None, unit, width, step, min, presets).and_then(SpinPick::value)
}

/// What a [`spin_field_auto`] returns: a value (points), or its Auto entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpinPick {
    Value(f64),
    Auto,
}

impl SpinPick {
    /// The value, `None` for Auto.
    pub fn value(self) -> Option<f64> {
        match self {
            SpinPick::Value(v) => Some(v),
            SpinPick::Auto => None,
        }
    }
}

/// [`spin_field`] whose dropdown starts with an Auto entry (Leading): `auto` is `Some(on)`, with
/// `on` checking it (no preset is then checked), or `None` for no Auto entry.
#[allow(clippy::too_many_arguments)]
pub fn spin_field_auto(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    value: Option<f64>,
    auto: Option<bool>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<SpinPick> {
    spin_generic(ui, value, auto, width, step, min, presets, &|v| unit.format(v), &mut |ui, fw| num_field(ui, id, value, unit, fw))
}

/// [`spin_field`] for unitless values (percent, degrees, 1/1000 em) with a display suffix; a
/// `None` value (the selection's values differ) shows blank.
#[allow(clippy::too_many_arguments)]
pub fn spin_plain(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    value: impl Into<Option<f64>>,
    suffix: &str,
    decimals: usize,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    let value = value.into();
    let fmt = |v: f64| format!("{v}{suffix}");
    spin_generic(ui, value, None, width, step, min, presets, &fmt, &mut |ui, fw| mixed_field(ui, id, value, suffix, decimals, fw))
        .and_then(SpinPick::value)
}

#[allow(clippy::too_many_arguments)]
fn spin_generic(
    ui: &mut Ui,
    value: Option<f64>,
    auto: Option<bool>,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
    fmt: &dyn Fn(f64) -> String,
    field: &mut dyn FnMut(&mut Ui, f32) -> Option<f64>,
) -> Option<SpinPick> {
    let t = Tokens::get(ui.ctx());
    let mut out = None;
    let h = 26.0;
    let enabled = ui.is_enabled();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (sr, sresp) = ui.allocate_exact_size(vec2(16.0, h), Sense::click());
        ui.painter().rect_filled(sr, CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, t.input);
        ui.painter().rect_stroke(sr, CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
        let up = Rect::from_min_max(sr.min, pos2(sr.right(), sr.center().y));
        let down = Rect::from_min_max(pos2(sr.left(), sr.center().y), sr.max);
        let hover = sresp.hover_pos();
        for (r, is_up) in [(up, true), (down, false)] {
            let c = if !enabled {
                t.text_disabled
            } else if hover.is_some_and(|p| r.contains(p)) {
                t.text_strong
            } else {
                t.icon
            };
            let m = r.center();
            let d = if is_up { -1.5 } else { 1.5 };
            ui.painter().line_segment([m + vec2(-3.0, -d), m + vec2(0.0, d)], Stroke::new(1.2, c));
            ui.painter().line_segment([m + vec2(0.0, d), m + vec2(3.0, -d)], Stroke::new(1.2, c));
        }
        // A blank field (values that differ) has nothing to step from.
        if sresp.clicked()
            && let Some(p) = sresp.interact_pointer_pos()
            && let Some(v) = value
        {
            let nv = if up.contains(p) { v + step } else { v - step };
            out = Some(SpinPick::Value(nv.max(min)));
        }
        let fw = if presets.is_empty() { width - 16.0 } else { width - 36.0 };
        if let Some(v) = field(ui, fw) {
            out = Some(SpinPick::Value(v.max(min)));
        }
        if !presets.is_empty() {
            let dresp = chevron_cell(ui, h);
            egui::Popup::menu(&dresp).show(|ui| {
                ui.set_min_width(width - 10.0);
                if let Some(on) = auto
                    && ui.selectable_label(on, tl!("Auto")).clicked()
                {
                    out = Some(SpinPick::Auto);
                }
                let current = value.filter(|_| auto != Some(true));
                for p in presets {
                    if ui.selectable_label(current.is_some_and(|v| (v - p).abs() < 1e-6), fmt(*p)).clicked() {
                        out = Some(SpinPick::Value(*p));
                    }
                }
            });
        }
    });
    out
}

/// A recessed search field with an italic `hint` (Layers' Search All, Swatches' Find), keeping its
/// text in egui memory under `id`. Returns the text.
pub fn search_field(ui: &mut Ui, id: egui::Id, hint: &str) -> String {
    let t = Tokens::get(ui.ctx());
    let mut query: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut query)
                    .id(id.with("edit"))
                    .frame(egui::Frame::NONE)
                    .hint_text(egui::RichText::new(tl!(hint)).italics())
                    .desired_width(ui.available_width()),
            );
        });
    ui.data_mut(|d| d.insert_temp(id, query.clone()));
    query
}

/// A number field that may be empty (Stroke dash/gap): `value` (points) shows in `unit` without
/// its suffix, to fit narrow fields. Returns `Some(new)` (points) on commit, where `new` is `None`
/// when the field was cleared; typed units and arithmetic work as in [`num_field`].
pub fn opt_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<f64>, unit: Unit, width: f32) -> Option<Option<f64>> {
    let t = Tokens::get(ui.ctx());
    let id = ui.id().with(id);
    let shown = value.map(|v| unit.number(v)).unwrap_or_default();
    let editing = ui.memory(|m| m.has_focus(id));
    let mut buf: String = if editing { ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.clone()) } else { shown.clone() };
    let enabled = ui.is_enabled();
    let framed = egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, if editing { t.accent } else { t.input_border }))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(4, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut buf)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_width(width - 10.0)
                    .font(egui::FontId::proportional(12.5))
                    .text_color(if enabled { t.text_strong } else { t.text_disabled }),
            )
        });
    let resp = framed.inner;
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step a value (an empty field stays empty); a drag on its label scrubs it.
    let stepped = step_value(ui, &resp, &mut buf, |b| Some(unit.from_pt(unit.parse(b)?)), |v| unit.number(unit.to_pt(v)));
    let scrubbed = scrub::field(ui, id, framed.response.rect, &buf, value.map(|v| unit.from_pt(v)), STEP_DECIMALS);
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    if resp.lost_focus() && buf != shown {
        let s = buf.trim();
        if s.is_empty() { Some(None) } else { typed_unit(ui.ctx(), unit).parse(s).map(Some) }
    } else {
        stepped.or(scrubbed).map(|v| Some(unit.to_pt(v)))
    }
}

/// A [`text_field`] with a preset list on its right (Export for Screens' sizes: `2x`, `100w`…).
/// Returns the new text when a change is committed (Enter or focus loss) or a preset is picked.
pub fn text_presets(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &str, presets: &[&str], width: f32) -> Option<String> {
    let mut out = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        out = text_field(ui, &id, Some(value), width - 20.0, 1);
        let resp = chevron_cell(ui, 26.0);
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(width - 10.0);
            for p in presets {
                if ui.selectable_label(*p == value, *p).clicked() {
                    out = Some((*p).to_string());
                }
            }
        });
    });
    out
}
