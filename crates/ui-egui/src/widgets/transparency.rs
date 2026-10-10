//! Blend-mode choices and shared opacity/blend editing.

use super::{BlendMode, Live, Sense, Stroke, StrokeKind, Tokens, Ui, combo, dim_label, icons, mixed_field, vec2};

/// Whether the blend-mode list draws a separator above `BlendMode::ALL[i]`: where a
/// [`BlendMode::group`] starts.
pub fn blend_separator_before(i: usize) -> bool {
    i > 0 && i < BlendMode::ALL.len() && BlendMode::ALL[i].group() != BlendMode::ALL[i - 1].group()
}

/// The blend-mode dropdown, its groups separated (Normal | darken | lighten | contrast | inversion
/// | component modes). `None` (objects that differ) shows blank. Returns the chosen mode.
pub fn blend_dropdown(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: Option<BlendMode>, width: f32) -> Option<BlendMode> {
    combo(ui, id, current.map_or("", |m| tl!(m.label())), width, false, |ui| {
        let mut chosen = None;
        for (i, m) in BlendMode::ALL.into_iter().enumerate() {
            if blend_separator_before(i) {
                ui.separator();
            }
            if ui.selectable_label(Some(m) == current, tl!(m.label())).clicked() {
                chosen = Some(m);
            }
        }
        chosen
    })
}

/// The blend mode an effect parameter names (`"mode": "multiply"`), which editors show as a
/// [`blend_param_dropdown`].
pub fn blend_param(key: &str, value: &serde_json::Value) -> Option<BlendMode> {
    value.as_str().filter(|_| key == "mode").and_then(BlendMode::parse)
}

/// [`blend_dropdown`] for a blend-mode parameter; returns the chosen mode's parameter value.
pub fn blend_param_dropdown(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: BlendMode) -> Option<serde_json::Value> {
    blend_dropdown(ui, id, Some(current), 120.0).map(|m| serde_json::Value::String(m.label().to_ascii_lowercase()))
}

/// An edit made with [`opacity_blend`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransparencyEdit {
    Blend(BlendMode),
    /// Opacity in percent, with the slider's drag phase (`Released` for a typed value).
    Opacity(f64, Live),
}

/// The transparency controls shared by the Transparency panel and the Appearance panel's Opacity
/// popups: the grouped blend-mode dropdown, the opacity percent field and its slider popup.
/// `opacity` (0..1) and `blend` are `None` where the objects differ (shown blank).
pub fn opacity_blend(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    opacity: Option<f32>,
    blend: Option<BlendMode>,
    enabled: bool,
) -> Option<TransparencyEdit> {
    let t = Tokens::get(ui.ctx());
    let mut edit = None;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(m) = blend_dropdown(ui, (id, "blend"), blend, 104.0) {
                edit = Some(TransparencyEdit::Blend(m));
            }
        });
        dim_label(ui, "Opacity:");
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(o) = mixed_field(ui, (id, "opacity"), opacity.map(|o| o as f64 * 100.0), "%", 0, 50.0) {
                edit = Some(TransparencyEdit::Opacity(o.clamp(0.0, 100.0), Live::Released));
            }
        });
        let (r, resp) = ui.allocate_exact_size(vec2(18.0, 26.0), if enabled { Sense::click() } else { Sense::hover() });
        ui.painter().rect_stroke(r, 2, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
        icons::paint(ui, "chevron-right", r.shrink2(vec2(3.0, 7.0)), if enabled { t.icon } else { t.text_disabled });
        egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let mut o = opacity.unwrap_or(1.0) * 100.0;
            let r = ui.add(egui::Slider::new(&mut o, 0.0..=100.0).show_value(false));
            let phase = if r.drag_stopped() || (r.changed() && !r.dragged()) {
                Live::Released
            } else if r.changed() {
                Live::Dragging
            } else {
                return;
            };
            edit = Some(TransparencyEdit::Opacity(o.round() as f64, phase));
        });
    });
    edit
}
