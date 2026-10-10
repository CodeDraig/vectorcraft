//! Menu invocation, dialog routing and focused-field command handling.

use super::{Value, VectorcraftApp, json, waits_for_ime};

/// What a menu click runs: items whose label ends with "…" and that carry default params open a
/// generic parameter dialog (fields = the defaults) instead of running immediately.
pub fn click_target(label: &str, id: &str, p: &Value) -> (String, Value) {
    let dialog = label.ends_with('…')
        && p.as_object().is_some_and(|o| !o.is_empty())
        && !matches!(id, "effect.dialog" | "window.panel" | "window.brightness" | "view.screenMode" | "plugin.dialog");
    if dialog {
        return ("ui.paramDialog".into(), json!({"command": id, "label": label.trim_end_matches('…'), "params": p}));
    }
    (id.to_string(), if p.is_null() { json!({}) } else { p.clone() })
}

/// The dialog of command `id` that shows the selection's current values (its query): its heading
/// and the queried values it doesn't show. Type on a Path Options leaves the brackets where they
/// are.
fn queried_dialog(id: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match id {
        // Whether the text overflows, and Shrink Text's factor, are read-only facts.
        "text.areaOptions" => ("Area Type Options", &["overflow", "fitScale"]),
        "type.pathOptions" => ("Type on a Path Options", &["start", "end"]),
        _ => return None,
    })
}

/// The dialog (kind, fields) the menu item of command `id` opens, as the reference app's does.
pub(super) fn menu_dialog(id: &str) -> Option<(&'static str, Value)> {
    Some(match id {
        "object.move" => ("move", json!({"dx": "0 pt", "dy": "0 pt"})),
        "object.rotate" => ("rotate", json!({"angle": 0})),
        "object.scale" => ("scale", json!({"sx": 100, "sy": 100, "uniform": true})),
        "object.transformEach" => (crate::dialogs::transform_each::KIND, crate::dialogs::transform_each::fields()),
        "object.reflect" => ("reflect", json!({"axis": "vertical"})),
        "object.shear" => ("shear", json!({"angle": 0, "axis": "horizontal"})),
        "path.average" => ("average", json!({"axis": "both"})),
        "object.path.offsetPath" => ("offsetPath", json!({"offset": "10 pt", "joins": "miter", "miterLimit": 4})),
        "object.path.simplify" => ("simplify", json!({"tolerance": "1 pt"})),
        "object.path.splitIntoGrid" => ("splitIntoGrid", json!({"rows": 2, "columns": 2, "gutter": "12 pt"})),
        "object.vectorHalftone" => (crate::dialogs::halftone::KIND, crate::dialogs::halftone::fields()),
        "artboard.rearrange" => (crate::dialogs::rearrange_artboards::KIND, crate::dialogs::rearrange_artboards::fields()),
        _ => return None,
    })
}

/// The event a focused text field takes for Edit-menu command `id` (Select All, Cut, Copy,
/// Paste), as the same keys would send it.
pub(super) fn field_event(app: &mut VectorcraftApp, id: &str) -> Option<egui::Event> {
    Some(match id {
        "select.all" => egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
        "edit.copy" => egui::Event::Copy,
        "edit.cut" => egui::Event::Cut,
        "edit.paste" | "edit.pasteWithoutFormatting" => egui::Event::Paste(app.system_clipboard_text()?),
        _ => return None,
    })
}

/// Invoke an item clicked in the system menu bar (macOS; its key equivalents come back to egui as
/// key presses, see [`crate::native_menu`]). While a text field has the keyboard, Select All, Cut,
/// Copy and Paste act on the field's text, as their keys do; everything else (and those commands
/// with no field focused) goes to [`invoke`].
pub fn invoke_from_system_menu(app: &mut VectorcraftApp, ctx: &egui::Context, id: &str, p: Value) {
    match focused_text_field(ctx).and_then(|f| Some((f, field_event(app, id)?))) {
        Some((field, e)) => to_field(ctx, field, e),
        None => invoke(app, id, p),
    }
}

/// The text field that has the keyboard, if any.
pub(super) fn focused_text_field(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.memory(|m| m.focused()).filter(|&id| egui::TextEdit::load_state(ctx, id).is_some())
}

/// Give text field `field` the keyboard (back) and event `e`, a [`field_event`], this frame.
pub(super) fn to_field(ctx: &egui::Context, field: egui::Id, e: egui::Event) {
    ctx.memory_mut(|m| m.request_focus(field));
    ctx.input_mut(|i| i.events.push(e));
}

/// Invoke a menu/command id with UI side effects (dialogs for "…" commands that need input).
pub fn invoke(app: &mut VectorcraftApp, id: &str, p: Value) {
    if waits_for_ime(app, id) {
        return;
    }
    if let Some((kind, fields)) = menu_dialog(id)
        && p.as_object().is_none_or(|o| o.is_empty())
    {
        app.ui.dialog = Some(crate::state::Dialog::new(kind, fields));
        return;
    }
    // Envelope Distort: Warp Options, Envelope Mesh and Envelope Options, from the selection.
    if crate::dialogs::envelope::opens(id) && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::envelope::open(app, id) {
            app.status(e);
        }
        return;
    }
    // Repeat Options: a dialog with the selected repeat's current values.
    if id == "object.repeat.options"
        && p.as_object().is_none_or(|o| o.is_empty())
        && let Some(fields) = crate::panels::pattern_options::repeat_fields(app)
    {
        let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Repeat Options", "params": fields}));
        return;
    }
    // Area Type Options and Type on a Path Options: dialogs with the selected type's current
    // values (the command's query), less those the dialog leaves alone.
    if let Some((label, hidden)) = queried_dialog(id)
        && p.as_object().is_none_or(|o| o.is_empty())
    {
        match app.session.execute(id, &json!({})) {
            Ok(mut fields) => {
                if let Some(o) = fields.as_object_mut() {
                    o.retain(|k, _| !hidden.contains(&k.as_str()));
                }
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": label, "params": fields}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // Object → Graph → Type… / Data…: dialogs with the selected graph's current values.
    if matches!(id, "graph.setType" | "graph.setData") && p.as_object().is_none_or(|o| o.is_empty()) {
        match app.session.execute(id, &json!({})) {
            Ok(v) => {
                let (label, fields) = if id == "graph.setData" { ("Graph Data", json!({"csv": v["csv"]})) } else { ("Graph Type", v) };
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": label, "params": fields}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // New View… / Edit Views…: name dialogs.
    if id == "view.saved.new" && p.as_object().is_none_or(|o| o.is_empty()) {
        let n = app.session.active().map_or(0, |d| d.doc.views.len()) + 1;
        let _ = app.run("ui.paramDialog", json!({"command": id, "label": "New View", "params": {"name": format!("View {n}")}}));
        return;
    }
    if id == "view.saved.edit" && p.as_object().is_none_or(|o| o.is_empty()) {
        let first = app.session.active().and_then(|d| d.doc.views.first().map(|v| v.name.clone()));
        match first {
            Some(name) => {
                let _ = app
                    .run("ui.paramDialog", json!({"command": id, "label": "Edit Views", "params": {"name": name, "newName": "", "delete": false}}));
            }
            None => app.status("No saved views (View → New View…)"),
        }
        return;
    }
    // Save Selection…: a name dialog, starting from the first free "Selection N".
    if id == "select.save" && p.as_object().is_none_or(|o| o.is_empty()) {
        let name = app.session.active().map(|d| vectorcraft_engine::doc::SavedSelection::default_name(&d.doc.saved_selections)).unwrap_or_default();
        let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Save Selection", "params": {"name": name}}));
        return;
    }
    // Edit Selection…: the saved selections in a list, to rename or delete.
    if id == "select.editSaved" && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::edit_selection::open(app) {
            app.status(e);
        }
        return;
    }
    // Document Raster Effects Settings and File Info: their dialogs.
    if matches!(id, "document.rasterEffectsSettings" | "file.info") && p.as_object().is_none_or(|o| o.is_empty()) {
        let r = if id == "file.info" { crate::dialogs::file_info::open(app) } else { crate::dialogs::raster_effects::open(app) };
        if let Err(e) = r {
            app.status(e);
        }
        return;
    }
    // Rasterize…: its options, starting from the document's raster effects settings.
    if id == "object.rasterize" && p.as_object().is_none_or(|o| o.is_empty()) {
        match app.session.execute("document.rasterEffectsSettings", &json!({})) {
            Ok(v) => {
                let params = json!({"ppi": v["resolution"], "colorModel": v["colorModel"], "background": v["background"], "antiAlias": if v["antiAlias"] == true { "art" } else { "none" }, "clippingMask": v["clippingMask"], "addAround": v["addAround"]});
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Rasterize", "params": params}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // Text Wrap Options: a dialog with the selected wrap object's current values.
    if id == "object.textWrap.options" && p.as_object().is_none_or(|o| o.is_empty()) {
        match app.session.execute(id, &json!({})) {
            Ok(fields) => {
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Text Wrap Options", "params": fields}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // Blend Options…: its dialog, on the selected blend's options.
    if id == "object.blend.options" && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::blend_options::open(app) {
            app.status(e);
        }
        return;
    }
    // Expand…: its dialog.
    if id == "object.expand" && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::expand::open(app) {
            app.status(e);
        }
        return;
    }
    // Help links: the command returns the URL; a menu click opens it (agents just get the URL).
    if matches!(id, "help.discord" | "help.website" | "help.appPage" | "help.github") {
        app.open_link(id);
        return;
    }
    if let Err(e) = app.run(id, p) {
        app.status(e);
    } else if matches!(id, "object.pattern.make" | "object.pattern.edit") {
        app.ui.open_panel = Some("patternOptions".into());
    }
}
