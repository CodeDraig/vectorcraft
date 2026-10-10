//! Dynamic menu labels, shortcut typography and translation strings.

use super::{
    Item, UI_COMMANDS, Value, VectorcraftApp, hidden_when_disabled, menu_tree, perspective_grid, recent_slot, saved_selection_name,
    saved_selection_slot, shortcut_of,
};

/// Label for toggles whose text flips (Outline/Preview, Hide/Show …).
pub fn dynamic_label(app: &VectorcraftApp, id: &str, label: &str) -> String {
    let v = &app.ui.view;
    match id {
        "view.outline" => if v.outline { "Preview" } else { "Outline" }.into(),
        "window.workspace.reset" => format!("Reset {}", app.ui.workspace),
        "view.edges" => if v.edges { "Hide Edges" } else { "Show Edges" }.into(),
        "view.cornerWidget" => if v.corner_widgets { "Hide Corner Widget" } else { "Show Corner Widget" }.into(),
        "view.textThreads" => if v.text_threads { "Hide Text Threads" } else { "Show Text Threads" }.into(),
        "type.hiddenCharacters" => if v.hidden_chars { "Hide Hidden Characters" } else { "Show Hidden Characters" }.into(),
        "view.gradientAnnotator" => if v.gradient_annotator { "Hide Gradient Annotator" } else { "Show Gradient Annotator" }.into(),
        "effect.last" => match &app.last_effect {
            Some((e, _)) => format!("Last Effect: {}", vectorcraft_effects::effect_info(e).map(|i| i.label).unwrap_or(e.as_str())),
            None => label.into(),
        },
        id if id.starts_with("type.recentFont") => {
            let n: usize = id["type.recentFont".len()..].parse().unwrap_or(0);
            n.checked_sub(1).and_then(|i| app.recent_fonts().get(i)).cloned().unwrap_or_else(|| "—".into())
        }
        id if id.starts_with("view.goto") => {
            let n: usize = id["view.goto".len()..].parse().unwrap_or(0);
            app.session.active().and_then(|d| n.checked_sub(1).and_then(|i| d.doc.views.get(i)).map(|v| v.name.clone())).unwrap_or_else(|| "—".into())
        }
        id if saved_selection_slot(id).is_some() => saved_selection_name(app, id).unwrap_or_else(|| "—".into()),
        "view.artboards" => if v.artboards { "Hide Artboards" } else { "Show Artboards" }.into(),
        "view.rulers" => if v.rulers { "Hide Rulers" } else { "Show Rulers" }.into(),
        "view.boundingBox" => if v.bounding_box { "Hide Bounding Box" } else { "Show Bounding Box" }.into(),
        "view.transparencyGrid" => {
            if app.session.active().is_some_and(|d| d.transparency_grid) { "Hide Transparency Grid" } else { "Show Transparency Grid" }.into()
        }
        "view.guides" => if v.guides { "Hide Guides" } else { "Show Guides" }.into(),
        "view.grid" => if v.grid { "Hide Grid" } else { "Show Grid" }.into(),
        "view.guides.lock" => if app.session.guides_locked() { "Unlock Guides" } else { "Lock Guides" }.into(),
        "view.slices.hide" => if app.session.slices_hidden() { "Show Slices" } else { "Hide Slices" }.into(),
        "view.printTiling" => if app.session.active().is_some_and(|d| d.print_tiling) { "Hide Print Tiling" } else { "Show Print Tiling" }.into(),
        id if id.starts_with("file.openRecent") => recent_slot(app, id)
            .map(|p| std::path::Path::new(p).file_name().map_or(p.clone(), |f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| "—".into()),
        // Edit Contents reads Edit Envelope while an envelope's contents are being edited.
        "object.envelope.editContents" => {
            let editing = app.session.active().is_some_and(|st| {
                st.selection
                    .objects
                    .first()
                    .and_then(|id| vectorcraft_doc::live::envelope_of(&st.doc, *id))
                    .is_some_and(|e| matches!(e.kind, vectorcraft_doc::NodeKind::Envelope { editing: true, .. }))
            });
            if editing { "Edit Envelope" } else { "Edit Contents" }.into()
        }
        "edit.undo" => app.session.active().and_then(|d| d.history.undo.last()).map(|h| format!("Undo {}", h.label)).unwrap_or_else(|| "Undo".into()),
        "edit.redo" => app.session.active().and_then(|d| d.history.redo.last()).map(|h| format!("Redo {}", h.label)).unwrap_or_else(|| "Redo".into()),
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => {
            crate::panels::swatches::user_library(app, id).map_or_else(|| "—".into(), |l| l.name)
        }
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => {
            crate::panels::graphic_styles::user_library(app, id).map_or_else(|| "—".into(), |l| l.name)
        }
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => {
            crate::dialogs::perspective_presets::slot_preset(app, id).unwrap_or_else(|| "—".into())
        }
        "perspective.grid.show" => if perspective_grid(app).is_some_and(|g| g.visible) { "Hide Grid" } else { "Show Grid" }.into(),
        "perspective.grid.rulers" => if perspective_grid(app).is_some_and(|g| g.rulers) { "Hide Rulers" } else { "Show Rulers" }.into(),
        "perspective.grid.lock" => if perspective_grid(app).is_some_and(|g| g.locked) { "Unlock Grid" } else { "Lock Grid" }.into(),
        _ => label.into(),
    }
}

/// Does the menu item `id` show a name that is user, file or system data rather than an interface
/// label: a recent file, a saved view, a font, a user library or preset, a custom workspace, a
/// plug-in? Such names are shown as they are, never translated (a workspace the user calls
/// "Layers" stays "Layers").
pub(super) fn shows_a_name(id: &str, label: &str) -> bool {
    const SLOTS: [&str; 6] = [
        "file.openRecent",
        "view.goto",
        "type.recentFont",
        crate::panels::swatches::USER_SLOT,
        crate::panels::graphic_styles::USER_SLOT,
        crate::dialogs::perspective_presets::SLOT,
    ];
    SLOTS.iter().any(|s| id.starts_with(s))
        || saved_selection_slot(id).is_some()
        || matches!(id, "text.setStyle" | "plugin.dialog")
        || (id == "window.workspace" && !crate::workspaces::is_builtin(label))
        || (matches!(id, "effect.apply" | "effect.dialog") && vectorcraft_effects::plugin_effects().iter().any(|e| e.label == label))
}

/// The label of a menu item as drawn: [`dynamic_label`] in the UI language. Labels assembled
/// around a name (Undo *Move*, Reset *Essentials*, Last Effect: *Drop Shadow*) are translated as
/// templates so the name can move; names that are user data (files, views, fonts, custom
/// workspaces, plug-ins: [`shows_a_name`]) pass through.
pub fn display_label(app: &VectorcraftApp, id: &str, label: &str) -> String {
    let lang = crate::i18n::current();
    let fmt = crate::i18n::fmt;
    match id {
        "edit.undo" | "edit.redo" => {
            let last = app.session.active().and_then(|d| if id == "edit.undo" { d.history.undo.last() } else { d.history.redo.last() });
            match last {
                Some(h) if id == "edit.undo" => fmt(tl!("Undo {name}"), &[("name", tl!(&h.label))]),
                Some(h) => fmt(tl!("Redo {name}"), &[("name", tl!(&h.label))]),
                None => tl!(if id == "edit.undo" { "Undo" } else { "Redo" }).to_string(),
            }
        }
        "window.workspace.reset" => {
            let name = &app.ui.workspace;
            fmt(tl!("Reset {name}"), &[("name", if crate::workspaces::is_builtin(name) { tl!(name) } else { name })])
        }
        "effect.last" => match &app.last_effect {
            Some((e, _)) => {
                let name = vectorcraft_effects::effect_info(e).map(|i| i.label).unwrap_or(e.as_str());
                fmt(tl!("Last Effect: {name}"), &[("name", tl!(name))])
            }
            None => crate::i18n::tr_id(lang, id, label).to_string(),
        },
        _ => {
            let shown = dynamic_label(app, id, label);
            if shows_a_name(id, &shown) { shown } else { crate::i18n::tr_id(lang, id, &shown).to_string() }
        }
    }
}

/// `Cmd+Shift+]` → `⇧⌘]` on macOS, `Ctrl+Shift+]` elsewhere.
pub fn pretty_shortcut(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        let mut mods = String::new();
        let parts: Vec<&str> = s.split('+').collect();
        let (key, ms): (&str, &[&str]) = if s.ends_with("++") {
            ("+", parts.get(..parts.len().saturating_sub(2)).unwrap_or_default())
        } else {
            parts.split_last().map_or(("", &[][..]), |(k, rest)| (*k, rest))
        };
        for m in ["Ctrl", "Alt", "Shift", "Cmd"] {
            if ms.contains(&m) {
                mods.push_str(match m {
                    "Ctrl" => "⌃",
                    "Alt" => "⌥",
                    "Shift" => "⇧",
                    _ => "⌘",
                });
            }
        }
        format!("{mods}{key}")
    } else {
        s.replace("Cmd", "Ctrl")
    }
}

/// The shortcut a menu item shows: its command's (items without params), or a panel's for the
/// Window menu's `window.panel {panel}` items.
pub fn item_shortcut(id: &str, p: &Value) -> Option<&'static str> {
    match (id, p.get("panel").and_then(Value::as_str)) {
        ("window.panel", Some(panel)) => crate::shortcut_editor::panel_shortcut(panel),
        _ if p.is_null() => shortcut_of(id),
        _ => None,
    }
}

/// Labels only the canvas context menu ([`context_items`]) shows, so the catalog tests can insist
/// every one of them is translated (a test checks the list against the menu).
pub const CONTEXT_LABELS: &[&str] = &[
    "Isolate Selected Group",
    "Exit Isolation Mode",
    "Make Clipping Mask",
    "Release Clipping Mask",
    "Make Compound Path",
    "Release Compound Path",
    "Select All",
];

/// The labels [`dynamic_label`] can show in place of an item's own (Show/Hide pairs and the like),
/// so the catalog tests can insist every one of them is translated.
pub const DYNAMIC_LABELS: &[&str] = &[
    "Preview",
    "Outline",
    "Hide Edges",
    "Show Edges",
    "Hide Corner Widget",
    "Show Corner Widget",
    "Hide Text Threads",
    "Show Text Threads",
    "Hide Hidden Characters",
    "Show Hidden Characters",
    "Hide Gradient Annotator",
    "Show Gradient Annotator",
    "Hide Artboards",
    "Show Artboards",
    "Hide Rulers",
    "Show Rulers",
    "Hide Bounding Box",
    "Show Bounding Box",
    "Hide Transparency Grid",
    "Show Transparency Grid",
    "Hide Guides",
    "Show Guides",
    "Hide Grid",
    "Show Grid",
    "Unlock Guides",
    "Lock Guides",
    "Show Slices",
    "Hide Slices",
    "Hide Print Tiling",
    "Show Print Tiling",
    "Edit Envelope",
    "Edit Contents",
    "Undo",
    "Redo",
    "Unlock Grid",
    "Lock Grid",
];

/// Every English string the menus, the command palette, the panel registry, the toolbar and the
/// Preferences dialog can show: what a complete language catalog has to cover. Names that are user
/// data (fonts, recent files, saved views, libraries) and the point sizes are left out.
pub fn menu_strings() -> std::collections::BTreeSet<String> {
    use std::collections::BTreeSet;
    fn user_data(id: &str) -> bool {
        hidden_when_disabled(id) || id.starts_with("type.recentFont")
    }
    fn walk(items: &[Item], under: &str, out: &mut BTreeSet<String>) {
        for it in items {
            match it {
                Item::Cmd(l, id, _) => {
                    let size = l.strip_suffix(" pt").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()));
                    // VectorCraft › Language lists each language by its own name.
                    let language_name = *id == "app.language" && crate::i18n::Lang::all().any(|lang| lang.name() == *l);
                    if !user_data(id) && !size && !language_name && *l != "—" {
                        out.insert(l.to_string());
                    }
                }
                Item::Todo(l, _) | Item::Header(l) => {
                    out.insert(l.to_string());
                }
                Item::Sub(l, children) => {
                    out.insert(l.to_string());
                    // Type › Font lists the installed families: names, not UI text.
                    if !(under == "Type" && *l == "Font") {
                        walk(children, l, out);
                    }
                }
                Item::Sep => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    for (title, items) in menu_tree() {
        out.insert(title.to_string());
        walk(&items, title, &mut out);
    }
    out.extend(crate::native_menu::MAC_LABELS.iter().map(|s| s.to_string()));
    out.extend(DYNAMIC_LABELS.iter().map(|s| s.to_string()));
    out.extend(CONTEXT_LABELS.iter().map(|s| s.to_string()));
    out.extend(UI_COMMANDS.iter().map(|c| c.1.to_string()));
    for c in vectorcraft_engine::cmd::command_specs() {
        out.insert(c.label.to_string());
        out.extend(c.menu.iter().map(|m| m.to_string()));
    }
    out.extend(crate::state::all_panels().map(|(_, label)| label.to_string()));
    out.extend(vectorcraft_tools::catalog::all_tools().map(|t| t.label.to_string()));
    out.extend(crate::toolbar::BASIC.iter().map(|c| c.0.to_string()));
    out.extend(vectorcraft_color::BlendMode::ALL.iter().map(|m| m.label().to_string()));
    out.extend(vectorcraft_effects::effect_catalog().iter().map(|e| e.label.to_string()));
    out.extend(vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES.iter().map(|c| c.to_string()));
    for sp in vectorcraft_engine::cmd::prefscmds::PREF_SPECS {
        out.insert(sp.label.to_string());
        if !sp.section.is_empty() {
            out.insert(sp.section.to_string());
        }
        if let vectorcraft_engine::cmd::prefscmds::PrefKind::Choice(opts) = sp.kind {
            out.extend(opts.iter().map(|o| o.1.to_string()));
        }
    }
    out.extend(crate::prefs_dialog::UI_FIELDS.iter().map(|f| f.2.to_string()));
    out.extend(crate::dialogs::button_labels().into_iter().map(str::to_string));
    out.extend(crate::chrome::hint_strings().into_iter().map(str::to_string));
    out.remove("");
    out
}
