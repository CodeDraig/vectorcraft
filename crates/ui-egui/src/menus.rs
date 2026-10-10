//! The menu bar (Illustrator's menu tree) and UI-level commands.
//!
//! Items bound to a command id run through [`VectorcraftApp::run`]. Items not implemented yet are
//! listed (disabled, with their shortcut) so the full surface is visible and discoverable; the
//! parity tracker drives them to "done".

#[cfg(test)]
use labels::shows_a_name;
#[cfg(test)]
use rendering::{menu_body, pointer_reaches_title};

mod catalog;
pub use catalog::UI_COMMANDS;
mod commands;
pub use commands::run_ui_command;
pub(crate) use commands::{normalize_panel, opt_bool};
mod tree;
pub(crate) use tree::PATH_EFFECTS;
pub use tree::{context_items, menu_tree, ruler_unit_items};
mod rendering;
pub use rendering::{context_menu_body, menu_bar, ruler_menu_body};
mod invoke;
pub use invoke::{click_target, invoke, invoke_from_system_menu};
use invoke::{field_event, focused_text_field, menu_dialog, to_field};
mod labels;
pub use labels::{CONTEXT_LABELS, DYNAMIC_LABELS, display_label, dynamic_label, item_shortcut, menu_strings, pretty_shortcut};

use serde::Serialize;
use serde_json::{Value, json};

use vectorcraft_engine::cmd::fileio::SaveMode;

use crate::VectorcraftApp;
use crate::io;
use crate::panels::character::Face;
use crate::state::{DockTab, next_zoom};
use crate::theme::{self, Brightness, Tokens};
use crate::widgets;

#[derive(Clone, Debug)]
pub enum Item {
    /// (label, command id, params)
    Cmd(&'static str, &'static str, Value),
    /// Not implemented yet: (label, shortcut)
    Todo(&'static str, &'static str),
    Sub(&'static str, Vec<Item>),
    Sep,
    /// Section header (disabled label, e.g. "Vector Effects").
    Header(&'static str),
}

fn c(label: &'static str, id: &'static str) -> Item {
    Item::Cmd(label, id, Value::Null)
}
fn cp(label: &'static str, id: &'static str, p: Value) -> Item {
    Item::Cmd(label, id, p)
}
fn todo(label: &'static str) -> Item {
    Item::Todo(label, "")
}
fn todos(label: &'static str, sc: &'static str) -> Item {
    Item::Todo(label, sc)
}
fn sub(label: &'static str, items: Vec<Item>) -> Item {
    Item::Sub(label, items)
}
/// Window → … Libraries until the code-generated libraries land (disabled entries).
fn library_placeholders() -> Vec<Item> {
    vec![todo("Built-in Libraries"), todo("User Defined"), Sep, todo("Other Library…")]
}
use Item::Sep;

/// Checked state for toggle items.
pub fn checked(app: &VectorcraftApp, id: &str, p: &Value) -> Option<bool> {
    if id == "app.language" {
        let lang = p.get("lang").and_then(Value::as_str).unwrap_or("");
        let pref = app.session.prefs.interface_language.as_str();
        return Some(if lang.eq_ignore_ascii_case("auto") {
            crate::i18n::Lang::from_code(pref).is_none()
        } else {
            crate::i18n::Lang::from_code(pref).is_some_and(|l| l.code().eq_ignore_ascii_case(lang))
        });
    }
    let v = &app.ui.view;
    // An item that can be on or off is `Some` whatever the document and selection (off with none):
    // the macOS menu bar can't change an item's kind without rebuilding.
    let selected_text = |onpath: bool| {
        let st = app.session.active()?;
        st.selection.objects.iter().find_map(|id| match st.doc.node(*id).map(|n| &n.kind) {
            Some(vectorcraft_engine::doc::NodeKind::Text(t)) if !onpath || matches!(t.kind, vectorcraft_engine::doc::TextKind::OnPath { .. }) => {
                Some(t)
            }
            _ => None,
        })
    };
    Some(match id {
        "type.orientation.vertical" | "type.orientation.horizontal" => {
            selected_text(false).is_some_and(|t| t.vertical == (id == "type.orientation.vertical"))
        }
        "view.outline" => v.outline,
        "view.pixelPreview" => v.pixel_preview,
        "view.trimView" => v.trim_view,
        "view.snapToPixel" => v.snap_to_pixel,
        // Type on a Path effect of the selected path type.
        "type.pathOptions" if p.get("effect").is_some() => {
            selected_text(true).is_some_and(|t| p.get("effect").and_then(Value::as_str) == Some(t.path_effect.id()))
        }
        "view.smartGuides" => v.smart_guides,
        "view.grid" => v.grid,
        "view.snapToGrid" => v.snap_to_grid,
        "view.snapToPoint" => v.snap_to_point,
        "view.rulers" => v.rulers,
        "window.control" => app.ui.control_bar,
        "window.toolbar" => app.ui.toolbar,
        "window.toolbarAdvanced" => app.ui.toolbar_advanced,
        "window.toolbarColumns" => app.ui.toolbar_double,
        "window.floatTools" => {
            let tool = p.get("tool").and_then(Value::as_str).unwrap_or("");
            app.ui.floating_flyouts.iter().any(|f| f.tools.iter().any(|id| id == tool))
        }
        "window.taskBar" => app.ui.task_bar,
        "window.taskBar.pin" => app.ui.task_bar_place.pinned,
        "window.panel" => {
            let panel = p.get("panel").and_then(Value::as_str).unwrap_or("");
            let canonical = normalize_panel(panel);
            match canonical.and_then(DockTab::from_id) {
                _ if canonical.is_some_and(|id| crate::floating::group_of(&app.ui, id).is_some()) => true,
                Some(tab) if !app.ui.dock_collapsed => crate::floating::shown_tab(&app.ui) == Some(tab),
                _ => canonical.is_some_and(|id| app.ui.open_panel.as_deref() == Some(id)),
            }
        }
        "window.collapseDock" => app.ui.dock_collapsed,
        "window.workspace" => p.get("name").and_then(Value::as_str) == Some(app.ui.workspace.as_str()),
        "window.brightness" => p.get("brightness").and_then(Value::as_str).and_then(Brightness::parse) == Some(app.ui.brightness),
        "view.slices.lock" => app.session.slices_locked(),
        "object.slice.clipToArtboard" => app.session.active().is_some_and(|d| d.doc.slices_clip_to_artboard),
        "view.proofColors" => vectorcraft_render::proof::view().proof_colors,
        "view.overprintPreview" => vectorcraft_render::proof::view().overprint,
        "view.proofSetup" => p.get("target").and_then(Value::as_str) == Some(vectorcraft_render::proof::view().setup.target.id().as_str()),
        "perspective.grid.snap" => perspective_grid(app).is_some_and(|g| g.snap),
        "perspective.grid.lockStation" => perspective_grid(app).is_some_and(|g| g.lock_station),
        "file.documentColorMode" => {
            let cmyk = app.session.active().is_some_and(|d| d.doc.color_mode == vectorcraft_engine::doc::ColorMode::Cmyk);
            p.get("mode").and_then(Value::as_str) == Some(if cmyk { "cmyk" } else { "rgb" })
        }
        // The ruler context menu lists the document units, the current one checked.
        "document.setup" => p.get("units").and_then(Value::as_str).and_then(vectorcraft_engine::doc::Unit::named) == Some(app.session.general_unit()),
        _ => return None,
    })
}

/// The active document's perspective grid (View → Perspective Grid toggles).
fn perspective_grid(app: &VectorcraftApp) -> Option<vectorcraft_tools::distort::perspective::PerspectiveGrid> {
    app.session.active().map(|d| vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&d.doc))
}

/// Menu slots that are left out while they have nothing to show (unused saved views, recent files,
/// User Defined swatch and graphic style libraries).
fn hidden_when_disabled(id: &str) -> bool {
    id.starts_with("view.goto")
        || saved_selection_slot(id).is_some()
        || id.starts_with("file.openRecent")
        || id.starts_with(crate::panels::swatches::USER_SLOT)
        || id.starts_with(crate::panels::graphic_styles::USER_SLOT)
        || id.starts_with(crate::dialogs::perspective_presets::SLOT)
}

/// Menu items another item stands in for right now: Envelope Distort's Reset with Warp and Reset
/// with Mesh take the place of Make with Warp and Make with Mesh while an envelope is selected.
fn swapped_out(app: &VectorcraftApp, id: &str) -> bool {
    let reset = match id {
        "object.envelope.makeWithWarp" | "object.envelope.makeWithMesh" => false,
        "object.envelope.resetWithWarp" | "object.envelope.resetWithMesh" => true,
        _ => return false,
    };
    enabled(app, "object.envelope.release") != reset
}

/// File → Open Recent Files slots (Preferences → File Handling shows 0–30 of them).
const RECENT_IDS: [&str; io::MAX_RECENT_FILES] = [
    "file.openRecent1",
    "file.openRecent2",
    "file.openRecent3",
    "file.openRecent4",
    "file.openRecent5",
    "file.openRecent6",
    "file.openRecent7",
    "file.openRecent8",
    "file.openRecent9",
    "file.openRecent10",
    "file.openRecent11",
    "file.openRecent12",
    "file.openRecent13",
    "file.openRecent14",
    "file.openRecent15",
    "file.openRecent16",
    "file.openRecent17",
    "file.openRecent18",
    "file.openRecent19",
    "file.openRecent20",
    "file.openRecent21",
    "file.openRecent22",
    "file.openRecent23",
    "file.openRecent24",
    "file.openRecent25",
    "file.openRecent26",
    "file.openRecent27",
    "file.openRecent28",
    "file.openRecent29",
    "file.openRecent30",
];

/// The saved selection a `select.recallN` slot names (none past the document's last).
fn saved_selection_name(app: &VectorcraftApp, id: &str) -> Option<String> {
    let n = saved_selection_slot(id)?.checked_sub(1)?;
    app.session.active()?.doc.saved_selections.get(n).map(|x| x.name.clone())
}

/// The recent file a `file.openRecentN` slot names (none past the preference's count).
fn recent_slot<'a>(app: &'a VectorcraftApp, id: &str) -> Option<&'a String> {
    let n: usize = id.strip_prefix("file.openRecent")?.parse().ok()?;
    io::recent_files(app).get(n.checked_sub(1)?)
}

/// How the menus show item `id` (params `p`) now: (enabled, checked: `Some` for an item that is on
/// or off), or `None` while they leave it out: an empty slot ([`hidden_when_disabled`]) or an item
/// another stands in for ([`swapped_out`]).
pub fn shown_state(app: &VectorcraftApp, id: &str, p: &Value) -> Option<(bool, Option<bool>)> {
    let en = enabled(app, id);
    if (!en && hidden_when_disabled(id)) || swapped_out(app, id) {
        return None;
    }
    Some((en, checked(app, id, p)))
}

/// One row of a menu as it shows now. The in-window menus draw these and the macOS menu bar
/// ([`crate::native_menu`]) is built from them, so the two can't drift.
pub enum Entry<'a> {
    Sep,
    /// A section header (a disabled label), in the UI language.
    Header(&'static str),
    /// A submenu: its label in the UI language, and its items.
    Sub(&'static str, &'a [Item]),
    Item(Row<'a>),
}

/// A menu item as it shows now ([`entry`]).
pub struct Row<'a> {
    /// The command and its params; `None` for an item not implemented yet (always disabled).
    pub command: Option<(&'static str, &'a Value)>,
    /// The tree's own English label, which [`click_target`] reads.
    pub source: &'static str,
    /// The label in the UI language, as it reads now (Undo *Move*, Show/Hide…, a recent file).
    pub label: std::borrow::Cow<'static, str>,
    /// The shortcut as the registry writes it (`Cmd+Shift+Z`), the user's overrides applied.
    pub shortcut: Option<&'static str>,
    pub enabled: bool,
    /// `Some` for an item that is on or off.
    pub checked: Option<bool>,
}

impl Row<'_> {
    /// What choosing it runs ([`click_target`]); `None` for an item not implemented yet.
    pub fn target(&self) -> Option<(String, Value)> {
        self.command.map(|(id, p)| click_target(self.source, id, p))
    }
}

/// Item `it` as the menus show it now, or `None` while they leave it out ([`shown_state`]).
pub fn entry<'a>(app: &VectorcraftApp, it: &'a Item) -> Option<Entry<'a>> {
    use crate::i18n::t;
    Some(match it {
        Item::Sep => Entry::Sep,
        Item::Header(h) => Entry::Header(t(h)),
        Item::Sub(label, children) => Entry::Sub(t(label), children),
        Item::Todo(label, sc) => Entry::Item(Row {
            command: None,
            source: label,
            label: t(label).into(),
            shortcut: Some(*sc).filter(|s| !s.is_empty()),
            enabled: false,
            checked: None,
        }),
        Item::Cmd(label, id, p) => {
            let (enabled, checked) = shown_state(app, id, p)?;
            Entry::Item(Row {
                command: Some((id, p)),
                source: label,
                label: display_label(app, id, label).into(),
                shortcut: item_shortcut(id, p),
                enabled,
                checked,
            })
        }
    })
}

/// Changes whenever a plug-in is installed or removed: Object › Plug-ins and Effect › Plug-ins
/// list them (the command palette's cache follows it).
pub fn plugin_revision() -> u64 {
    vectorcraft_plugins::registry::revision()
}

/// View → Perspective Grid → One/Two/Three Point Perspective: the saved-preset slots of each type.
const PERSPECTIVE_SLOTS: [[&str; crate::dialogs::perspective_presets::SLOTS]; 3] = [
    [
        "ui.perspectiveUserPreset1.1",
        "ui.perspectiveUserPreset1.2",
        "ui.perspectiveUserPreset1.3",
        "ui.perspectiveUserPreset1.4",
        "ui.perspectiveUserPreset1.5",
    ],
    [
        "ui.perspectiveUserPreset2.1",
        "ui.perspectiveUserPreset2.2",
        "ui.perspectiveUserPreset2.3",
        "ui.perspectiveUserPreset2.4",
        "ui.perspectiveUserPreset2.5",
    ],
    [
        "ui.perspectiveUserPreset3.1",
        "ui.perspectiveUserPreset3.2",
        "ui.perspectiveUserPreset3.3",
        "ui.perspectiveUserPreset3.4",
        "ui.perspectiveUserPreset3.5",
    ],
];

/// Select → saved selections: the n-th saved selection of the active document.
const SAVED_SELECTION_IDS: [&str; vectorcraft_engine::doc::SavedSelection::MAX] = [
    "select.recall1",
    "select.recall2",
    "select.recall3",
    "select.recall4",
    "select.recall5",
    "select.recall6",
    "select.recall7",
    "select.recall8",
    "select.recall9",
    "select.recall10",
    "select.recall11",
    "select.recall12",
    "select.recall13",
    "select.recall14",
    "select.recall15",
    "select.recall16",
    "select.recall17",
    "select.recall18",
    "select.recall19",
    "select.recall20",
    "select.recall21",
    "select.recall22",
    "select.recall23",
    "select.recall24",
    "select.recall25",
];

/// The 1-based slot a `select.recallN` id stands for.
fn saved_selection_slot(id: &str) -> Option<usize> {
    id.strip_prefix("select.recall")?.parse().ok()
}

/// Type → Recent Fonts slots (Preferences › Type › Number of Recent Fonts shows up to 15).
const RECENT_FONT_IDS: [&str; crate::MAX_RECENT_FONTS] = [
    "type.recentFont1",
    "type.recentFont2",
    "type.recentFont3",
    "type.recentFont4",
    "type.recentFont5",
    "type.recentFont6",
    "type.recentFont7",
    "type.recentFont8",
    "type.recentFont9",
    "type.recentFont10",
    "type.recentFont11",
    "type.recentFont12",
    "type.recentFont13",
    "type.recentFont14",
    "type.recentFont15",
];

/// Effective shortcut of a command: the user's override (Edit → Keyboard Shortcuts) or the default.
pub fn shortcut_of(id: &str) -> Option<&'static str> {
    crate::shortcut_editor::command_shortcut(id)
}

/// Is a command currently enabled?
/// Commands that would act on the text being typed, held back while an IME composes in the Type
/// tool (its marked text isn't committed yet): Undo/Redo — the native menu takes ⌘Z ahead of the
/// IME —, the clipboard and the selection.
pub(crate) fn waits_for_ime(app: &VectorcraftApp, id: &str) -> bool {
    app.session.tool_composing()
        && (matches!(
            id,
            "edit.undo" | "edit.redo" | "edit.cut" | "edit.copy" | "edit.clear" | "edit.duplicate" | "select.all" | "select.none" | "select.inverse"
        ) || id.starts_with("edit.paste"))
}

pub fn enabled(app: &VectorcraftApp, id: &str) -> bool {
    if waits_for_ime(app, id) {
        return false;
    }
    if let Some(c) = vectorcraft_engine::find_command(id) {
        // The system clipboard's contents can be pasted with an empty internal clipboard.
        return (c.enabled)(&app.session).is_ok() || app.system_paste && id.starts_with("edit.paste");
    }
    match id {
        // Save is off for a clean document that already has its own file.
        "file.save" => app.session.active().is_some_and(|d| d.path.is_none() || d.converted || d.is_dirty()),
        "type.bold" | "type.italic" => crate::panels::character::text_style(app).is_some(),
        "file.reveal" => app.services.reveal.is_some() && app.session.active().is_some_and(|d| d.path.is_some()),
        "file.place"
        | "file.export.svg"
        | "file.export.png"
        | "file.exportAs"
        | "file.exportForScreens"
        | "file.documentSetup"
        | "view.zoomIn"
        | "view.zoomOut"
        | "view.fitArtboard"
        | "view.fitAll"
        | "view.actualSize" => app.session.active().is_some(),
        id if id.starts_with("file.openRecent") => recent_slot(app, id).is_some(),
        "file.clearRecent" => !app.ui.recent_files.is_empty(),
        id if id.starts_with("type.recentFont") => {
            id["type.recentFont".len()..].parse::<usize>().is_ok_and(|n| n >= 1 && n <= app.recent_fonts().len()) && app.session.active().is_some()
        }
        id if saved_selection_slot(id).is_some() => saved_selection_name(app, id).is_some(),
        id if id.starts_with("view.goto") => {
            id["view.goto".len()..].parse::<usize>().is_ok_and(|n| n >= 1 && app.session.active().is_some_and(|d| n <= d.doc.views.len()))
        }
        "effect.dialog" | "ui.recolorDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.cropImage" => selected_image(app, |_| true),
        "effect.applyLast" | "effect.last" => app.last_effect.is_some() && app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "file.export.pdf" | "ui.savePdfDialog" | "ui.fileInfoDialog" | "ui.rasterEffectsSettingsDialog" => app.session.active().is_some(),
        "ui.swatchOptions" | "ui.newSwatch" | "ui.newColorGroup" => app.session.active().is_some(),
        "ui.graphicStyleOptions" => app.session.active().is_some(),
        "ui.colorBalanceDialog" | "ui.saturateDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.saveSwatchLibrary" => app.session.active().is_some(),
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => crate::panels::swatches::user_library(app, id).is_some(),
        "ui.flattenTransparencyDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.saveGraphicStyleLibrary" => app.session.active().is_some(),
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => crate::panels::graphic_styles::user_library(app, id).is_some(),
        "ui.expandDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.spotColors" => app.session.active().is_some(),
        "ui.menuDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.dxfOptionsDialog" => app.session.active().is_some(),
        "links.editOriginal" | "links.reveal" => selected_image(app, |im| im.link.is_some()) || selected_placed(app),
        "ui.placementOptionsDialog" => selected_image(app, |_| true) || selected_placed(app),
        "ui.packageDialog" | "docInfo.save" => app.session.active().is_some(),
        "ui.epsOptionsDialog" => app.session.active().is_some(),
        "file.saveForWeb" | "file.saveForWeb.browser" => app.session.active().is_some(),
        "file.exportSelection" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "css.copy" | "css.exportFile" => app.session.active().is_some(),
        "print.printerSetup" => app.services.print.as_ref().is_some_and(|s| s.has_setup()),
        "plugin.dialog" => app.session.active().is_some(),
        "ui.perspectiveGridDialog" | "ui.savePerspectivePreset" => app.session.active().is_some(),
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => {
            app.session.active().is_some() && crate::dialogs::perspective_presets::slot_preset(app, id).is_some()
        }
        "ui.perspectivePlane" => app.session.active().is_some(),
        _ => true,
    }
}

/// Is a placed document selected?
fn selected_placed(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::PlacedDocument(_))))
    })
}

/// Is an image `keep` accepts selected?
fn selected_image(app: &VectorcraftApp, keep: impl Fn(&vectorcraft_doc::ImageObject) -> bool) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(&n.kind, vectorcraft_doc::NodeKind::Image(im) if keep(im))))
    })
}

/// What the Home screen remembers when it opens (`app.home`): the active document and the
/// document count. When either changes, the Home screen gives way to the document.
pub(crate) fn home_key(app: &VectorcraftApp) -> (Option<u64>, usize) {
    (app.session.active().map(|d| d.uid), app.session.documents().len())
}

/// Whether the Home screen is up: chosen with the Home button (`app.home`), or no document is open
/// and Preferences › General › Show The Home Screen When No Documents Are Open is on (#394). Off,
/// an app with no document shows an empty window, and the Home button still opens the screen.
pub(crate) fn home_showing(app: &VectorcraftApp) -> bool {
    app.ui.home.is_some() || (app.session.active().is_none() && app.session.prefs.show_home_screen)
}

#[derive(Serialize)]
pub struct MenuEntry {
    pub path: Vec<String>,
    pub label: String,
    pub command: Option<String>,
    pub params: Value,
    pub enabled: bool,
    pub shortcut: String,
}

/// Flattened menu for `ui.menu.list`.
pub fn menu_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    for (title, items) in menu_tree() {
        flatten(app, vec![title.to_string()], &items, &mut out);
    }
    out
}

/// Flattened canvas context menu for `ui.contextMenu.list` (paths are its submenus).
pub fn context_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    flatten(app, vec![], &context_items(app), &mut out);
    out
}

fn flatten(app: &VectorcraftApp, path: Vec<String>, items: &[Item], out: &mut Vec<MenuEntry>) {
    for it in items {
        match it {
            Item::Cmd(l, id, p) => {
                let Some((enabled, _)) = shown_state(app, id, p) else { continue };
                out.push(MenuEntry {
                    path: path.clone(),
                    label: dynamic_label(app, id, l),
                    command: Some(id.to_string()),
                    params: p.clone(),
                    enabled,
                    shortcut: item_shortcut(id, p).or_else(|| shortcut_of(id)).unwrap_or("").to_string(),
                })
            }
            Item::Todo(l, sc) => out.push(MenuEntry {
                path: path.clone(),
                label: l.to_string(),
                command: None,
                params: Value::Null,
                enabled: false,
                shortcut: sc.to_string(),
            }),
            Item::Sub(l, ch) => {
                let mut p = path.clone();
                p.push(l.to_string());
                flatten(app, p, ch, out);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One headless frame of the in-window menu bar; returns its titles (left to right) as
    /// (rect, id of the title's popup).
    fn bar_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(egui::Rect, egui::Id)> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
        let mut layer = None;
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
            layer = Some(ui.layer_id());
            menu_bar(app, ui);
        });
        out.textures_delta.clear();
        let layer = layer.unwrap();
        let mut titles: Vec<(egui::Rect, egui::Id)> = ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .get_layer(layer)
                .filter(|w| w.sense.senses_click() && w.rect.top() < 30.0)
                // `egui::Popup::default_response_id` of the title's response.
                .map(|w| (w.rect, w.id.with("popup")))
                .collect()
        });
        titles.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
        titles
    }

    fn open_titles(ctx: &egui::Context, titles: &[(egui::Rect, egui::Id)]) -> Vec<usize> {
        (0..titles.len()).filter(|&i| egui::Popup::is_id_open(ctx, titles[i].1)).collect()
    }

    #[test]
    fn hovering_another_title_switches_the_open_menu() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        bar_frame(&mut app, &ctx, vec![]);
        let titles = bar_frame(&mut app, &ctx, vec![]);
        assert_eq!(titles.len(), menu_tree().len());
        // 0 is the app menu, then File, Edit, Object.
        let (file, edit, object) = (titles[1].0.center(), titles[2].0.center(), titles[3].0.center());
        let frames = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut t = bar_frame(app, &ctx, events);
            for _ in 0..2 {
                t = bar_frame(app, &ctx, vec![]);
            }
            t
        };
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };

        // Nothing open: hovering a title opens nothing.
        let t = frames(&mut app, vec![egui::Event::PointerMoved(edit)]);
        assert!(open_titles(&ctx, &t).is_empty());

        // Click File, then move onto Edit and on to Object: each opens in turn, alone.
        frames(&mut app, vec![egui::Event::PointerMoved(file)]);
        frames(&mut app, vec![button(file, true)]);
        let t = frames(&mut app, vec![button(file, false)]);
        assert_eq!(open_titles(&ctx, &t), vec![1], "a click opens File");
        let t = frames(&mut app, vec![egui::Event::PointerMoved(edit)]);
        assert_eq!(open_titles(&ctx, &t), vec![2], "hovering Edit opens it and closes File");
        let t = frames(&mut app, vec![egui::Event::PointerMoved(object)]);
        assert_eq!(open_titles(&ctx, &t), vec![3], "hovering Object opens it and closes Edit");

        // A pointer resting on a title doesn't switch: File opened another way (the keyboard)
        // stays open while the pointer stays still over Object.
        egui::Popup::open_id(&ctx, t[1].1);
        let t = frames(&mut app, vec![]);
        assert_eq!(open_titles(&ctx, &t), vec![1], "a still pointer leaves the open menu alone");
    }

    #[test]
    fn a_popup_covering_a_title_does_not_switch_menus() {
        let ctx = egui::Context::default();
        let p = egui::pos2(90.0, 12.0);
        let mut reaches = (true, false);
        // The popup becomes hit-testable once egui has laid it out: draw a few frames.
        for _ in 0..3 {
            let raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let ctx = ui.ctx().clone();
                let covered = ui.interact(egui::Rect::from_center_size(p, egui::vec2(80.0, 24.0)), egui::Id::new("covered"), egui::Sense::click());
                let q = p + egui::vec2(0.0, 100.0);
                let free = ui.interact(egui::Rect::from_center_size(q, egui::vec2(80.0, 24.0)), egui::Id::new("free"), egui::Sense::click());
                egui::Area::new(egui::Id::new("popup")).order(egui::Order::Foreground).fixed_pos(p - egui::vec2(10.0, 10.0)).show(&ctx, |ui| {
                    ui.allocate_space(egui::vec2(20.0, 20.0));
                });
                reaches = (pointer_reaches_title(&ctx, &covered, p), pointer_reaches_title(&ctx, &free, q));
            });
            out.textures_delta.clear();
        }
        assert_eq!(reaches, (false, true));
    }

    #[test]
    fn names_in_menus_are_not_interface_labels() {
        for (id, label) in [
            ("file.openRecent1", "Layers.svg"),
            ("view.goto2", "Layers"),
            ("type.recentFont1", "Regular"),
            ("text.setStyle", "Black"),
            ("window.workspace", "Layers"),
            ("plugin.dialog", "Group"),
            ("window.userSwatchLibrary3", "Default"),
        ] {
            assert!(shows_a_name(id, label), "{id} {label}");
        }
        for (id, label) in [("object.group", "Group"), ("window.workspace", "Essentials"), ("effect.apply", "Drop Shadow")] {
            assert!(!shows_a_name(id, label), "{id} {label}");
        }
    }

    #[test]
    fn last_effect_dialog_and_view_toggles() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        assert!(!enabled(&app, "effect.last"));
        app.last_effect = Some(("distort.roughen".into(), json!({"size": 9})));
        assert!(enabled(&app, "effect.last"));
        assert_eq!(dynamic_label(&app, "effect.last", "Last Effect…"), "Last Effect: Roughen…");
        app.run("effect.last", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("__effect"), d.f64("size", 0.0)), ("effect", "distort.roughen".to_string(), 9.0));
        for (id, on, off) in [
            ("view.textThreads", "Hide Text Threads", "Show Text Threads"),
            ("view.gradientAnnotator", "Hide Gradient Annotator", "Show Gradient Annotator"),
            ("type.hiddenCharacters", "Show Hidden Characters", "Hide Hidden Characters"),
        ] {
            assert_eq!(dynamic_label(&app, id, ""), on);
            app.run(id, json!({})).unwrap();
            assert_eq!(dynamic_label(&app, id, ""), off);
        }
    }

    #[test]
    fn panel_ids_are_case_insensitive_and_labels_work() {
        assert_eq!(normalize_panel("layers"), Some("layers"));
        assert_eq!(normalize_panel("Layers"), Some("layers"));
        assert_eq!(normalize_panel("LAYERS"), Some("layers"));
        assert_eq!(normalize_panel("Swatches"), Some("swatches"));
        assert_eq!(normalize_panel("swatches"), Some("swatches"));
        assert_eq!(normalize_panel("Color Guide"), Some("colorGuide"));
        assert_eq!(normalize_panel("colorguide"), Some("colorGuide"));
        assert_eq!(normalize_panel("  Stroke  "), Some("stroke"));
        assert_eq!(normalize_panel("nope"), None);

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.run("window.panel", json!({"panel": "Layers"})).unwrap();
        assert_eq!(app.ui.dock_tab, crate::state::DockTab::Layers);
        app.run("window.panel", json!({"panel": "Swatches"})).unwrap();
        assert_eq!(app.ui.open_panel.as_deref(), Some("swatches"));
        assert!(checked(&app, "window.panel", &json!({"panel": "Swatches"})).unwrap());
        assert!(app.run("window.panel", json!({"panel": "nope"})).is_err());
        // A collapsed dock pops the panel out of its icon, by canonical id.
        app.ui.dock_collapsed = true;
        app.run("window.panel", json!({"panel": "Layers"})).unwrap();
        assert_eq!(app.ui.open_panel.as_deref(), Some("layers"));
        assert!(checked(&app, "window.panel", &json!({"panel": "Layers"})).unwrap());
        app.ui.dock_collapsed = false;
    }

    #[test]
    fn saved_views_store_and_restore_the_view() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        {
            let v = app.view_mut().unwrap();
            v.zoom = 3.0;
            v.center = vectorcraft_geom::Point::new(120.0, 80.0);
        }
        app.run("view.saved.new", json!({"name": "Detail"})).unwrap();
        assert!(enabled(&app, "view.goto1") && !enabled(&app, "view.goto2"));
        assert_eq!(dynamic_label(&app, "view.goto1", ""), "Detail");
        app.view_mut().unwrap().zoom = 0.5;
        app.run("view.goto1", json!({})).unwrap();
        let v = app.view().unwrap();
        assert_eq!((v.zoom, v.center.x, v.center.y), (3.0, 120.0, 80.0));
        // Unused slots stay out of the menu listing agents see.
        assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("view.goto2")));
    }

    fn app_with_a_rectangle() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app
    }

    #[test]
    fn save_selection_asks_for_a_name() {
        let mut app = app_with_a_rectangle();
        invoke(&mut app, "select.save", json!({}));
        let d = app.ui.dialog.clone().expect("Save Selection dialog");
        assert_eq!(
            (d.kind.as_str(), d.str("__command"), d.str("__label"), d.str("name")),
            ("command", "select.save".into(), "Save Selection".into(), "Selection 1".into())
        );
        // OK saves under the name typed.
        let mut d = d;
        d.fields.insert("name".into(), json!("Logo"));
        app.ui.dialog = Some(d);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.run("select.savedList", json!({})).unwrap(), json!(["Logo"]));
        // The next one starts from the first free name.
        invoke(&mut app, "select.save", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("name")), Some("Selection 1".into()));
    }

    #[test]
    fn saved_selections_are_listed_at_the_bottom_of_the_select_menu() {
        let mut app = app_with_a_rectangle();
        app.run("shape.ellipse", json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
        // Nothing saved: no slot is listed or enabled.
        assert!(!enabled(&app, "select.recall1"));
        assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall1")));
        app.run("select.save", json!({"name": "Ellipse"})).unwrap();
        app.run("select.none", json!({})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("select.save", json!({"name": "Everything"})).unwrap();
        assert!(enabled(&app, "select.recall1") && enabled(&app, "select.recall2") && !enabled(&app, "select.recall3"));
        assert_eq!(dynamic_label(&app, "select.recall2", "Saved Selection"), "Everything");
        // A name is shown as it is, not translated.
        assert_eq!(display_label(&app, "select.recall2", "Saved Selection"), "Everything");
        assert!(menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall2")));
        assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall3")));
        // Choosing one selects its objects again.
        app.run("select.none", json!({})).unwrap();
        let r = app.run("select.recall2", json!({})).unwrap();
        assert!(r["count"].as_u64().unwrap() >= 1);
        assert_eq!(app.session.active().unwrap().selection.len(), 2);
        // A deleted selection's slot leaves the menus.
        app.run("select.editSaved", json!({"name": "Ellipse", "delete": true})).unwrap();
        assert!(!enabled(&app, "select.recall2"));
        assert!(shown_state(&app, "select.recall2", &Value::Null).is_none());
        assert!(shown_state(&app, "select.recall1", &Value::Null).is_some());
    }

    #[test]
    fn edit_selection_opens_its_dialog_on_the_saved_selections() {
        let mut app = app_with_a_rectangle();
        // Nothing saved yet: the item is off, and invoking it (palette, shortcut) only says so.
        assert!(!enabled(&app, "select.editSaved"));
        invoke(&mut app, "select.editSaved", json!({}));
        assert!(app.ui.dialog.is_none());
        assert!(app.ui.status.contains("No saved selections"), "{}", app.ui.status);
        app.run("select.save", json!({"name": "Logo"})).unwrap();
        assert!(enabled(&app, "select.editSaved"));
        invoke(&mut app, "select.editSaved", json!({}));
        let d = app.ui.dialog.clone().expect("Edit Selection dialog");
        assert_eq!((d.kind.as_str(), d.str("orig0"), d.str("name0")), (crate::dialogs::edit_selection::KIND, "Logo".into(), "Logo".into()));
    }

    #[test]
    fn recent_fonts_and_corner_widget_toggle() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        let fams = vectorcraft_text::FontDb::global().families();
        let (a, b) = (fams[0].clone(), fams[fams.len() - 1].clone());
        app.run("text.create", json!({"x": 10, "y": 10, "text": "Hi"})).unwrap();
        app.run("text.setStyle", json!({"font": a})).unwrap();
        app.run("text.setStyle", json!({"font": b})).unwrap();
        assert_eq!(app.ui.recent_fonts, vec![b.clone(), a.clone()]);
        assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), a);
        assert!(enabled(&app, "type.recentFont2") && !enabled(&app, "type.recentFont3"));
        app.run("type.recentFont2", json!({})).unwrap();
        assert_eq!(app.ui.recent_fonts[0], a);
        assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Hide Corner Widget");
        app.run("view.cornerWidget", json!({})).unwrap();
        assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Show Corner Widget");
    }

    /// Selection & Anchor Display › Zoom to Selection (#394): Zoom In and Zoom Out bring the
    /// selection to the middle; off, they keep the view's centre.
    #[test]
    fn zoom_to_selection_centres_the_selection() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 40, "height": 20})).unwrap();
        let center = |app: &VectorcraftApp| app.view().unwrap().center;
        let start = center(&app);
        app.run("prefs.set", json!({"key": "zoomToSelection", "value": false})).unwrap();
        app.run("view.zoomIn", json!({})).unwrap();
        assert_eq!(center(&app), start, "off: the view's centre stays");
        app.run("prefs.set", json!({"key": "zoomToSelection", "value": true})).unwrap();
        app.run("view.zoomOut", json!({})).unwrap();
        assert_eq!(center(&app), vectorcraft_geom::Point::new(30.0, 30.0), "on: the selection's centre");
    }

    /// Preferences › Type › Number of Recent Fonts (#394): Type › Recent Fonts lists that many.
    #[test]
    fn number_of_recent_fonts_sets_how_many_are_listed() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.ui.recent_fonts = (1..=15).map(|i| format!("Font {i}")).collect();
        let listed = |app: &VectorcraftApp| RECENT_FONT_IDS.iter().filter(|id| enabled(app, id)).count();
        assert_eq!(listed(&app), 10, "10 by default");
        for n in [15, 2] {
            app.run("prefs.set", json!({"key": "recentFontsCount", "value": n})).unwrap();
            assert_eq!(listed(&app), n);
        }
        assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), "Font 2");
        assert!(app.run("type.recentFont3", json!({})).is_err(), "past the count");
    }

    #[test]
    fn recent_files_track_opens_and_saves() {
        let dir = std::env::temp_dir().join(format!("dc-recent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = VectorcraftApp::new(
            vectorcraft_engine::Session::new(),
            crate::Services {
                read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
                write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
                ..Default::default()
            },
        );
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let (a, b) = (dir.join("a.vectorcraft"), dir.join("b.vectorcraft"));
        for p in [&a, &b, &a] {
            app.run("file.saveAs", json!({"path": p.to_string_lossy()})).unwrap();
        }
        assert_eq!(app.ui.recent_files, [a.to_string_lossy(), b.to_string_lossy()]);
        assert_eq!(dynamic_label(&app, "file.openRecent2", ""), "b.vectorcraft");
        assert!(enabled(&app, "file.openRecent2") && !enabled(&app, "file.openRecent3"));
        app.run("file.openRecent2", json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.ui.recent_files[0], b.to_string_lossy(), "reopening moves it to the top");
        app.run("file.clearRecent", json!({})).unwrap();
        assert!(app.ui.recent_files.is_empty() && app.run("file.openRecent1", json!({})).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_bound_menu_command_exists() {
        fn walk(items: &[Item], bad: &mut Vec<String>) {
            for it in items {
                match it {
                    Item::Cmd(_, id, _)
                        if vectorcraft_engine::find_command(id).is_none()
                            && !UI_COMMANDS.iter().any(|c| c.0 == *id)
                            && !id.starts_with("object.path.")
                            && *id != "type.createOutlines" =>
                    {
                        bad.push(id.to_string());
                    }
                    Item::Sub(_, ch) => walk(ch, bad),
                    _ => {}
                }
            }
        }
        let mut bad = vec![];
        for (_, items) in menu_tree() {
            walk(&items, &mut bad);
        }
        assert!(bad.is_empty(), "menu items bound to unknown commands: {bad:?}");
    }

    #[test]
    fn shortcut_pretty() {
        if cfg!(target_os = "macos") {
            assert_eq!(pretty_shortcut("Cmd+Shift+]"), "⇧⌘]");
            assert_eq!(pretty_shortcut("Cmd+Alt+2"), "⌥⌘2");
        }
        assert_eq!(pretty_shortcut(""), "");
    }

    /// #543: Object › Image Trace › Make and Make and Expand trace with the Default preset at once,
    /// with no form asking for a preset's name.
    #[test]
    fn image_trace_make_traces_with_the_default_preset() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        for cmd in ["imageTrace.make", "imageTrace.makeAndExpand"] {
            let e = menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some(cmd)).unwrap();
            assert_eq!(e.path, ["Object", "Image Trace"]);
            assert_eq!(click_target(&e.label, cmd, &e.params), (cmd.to_string(), json!({"preset": "Default"})), "{}", e.label);
        }
    }

    #[test]
    fn expand_appearance_is_an_object_menu_item_only() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        let paths: Vec<Vec<String>> =
            menu_entries(&app).into_iter().filter(|e| e.command.as_deref() == Some("effect.expandAppearance")).map(|e| e.path).collect();
        assert_eq!(paths, [vec!["Object".to_string()]]);
    }

    #[test]
    fn menus_are_as_wide_as_their_items() {
        // Items size the popup to the widest label and shortcut instead of the widest a menu may be.
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
        let tree = menu_tree();
        let (_, file) = tree.iter().find(|(t, _)| *t == "File").unwrap();
        let id = egui::Id::new("test-menu");
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::Area::new(id).show(ui.ctx(), |ui| {
                    egui::containers::menu::menu_style(ui.style_mut());
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| menu_body(&app, ui, file, &mut None));
                });
            });
            out.textures_delta.clear();
        }
        let w = ctx.memory(|m| m.area_rect(id)).unwrap().width();
        assert!((230.0..340.0).contains(&w), "File menu is {w} pt wide");
    }

    #[test]
    fn long_menus_scroll_inside_a_short_window() {
        // The Window menu is taller than a 600 pt window: it stops above the bottom edge and
        // scrolls instead of running off screen.
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let tree = menu_tree();
        let (_, window) = tree.iter().find(|(t, _)| *t == "Window").unwrap();
        let id = egui::Id::new("test-long-menu");
        let rect = |height: f32| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, height));
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                let mut out = ctx.run_ui(input, |ui| {
                    egui::Area::new(id).fixed_pos(egui::pos2(300.0, 30.0)).show(ui.ctx(), |ui| {
                        egui::containers::menu::menu_style(ui.style_mut());
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| menu_body(&app, ui, window, &mut None));
                    });
                });
                out.textures_delta.clear();
            }
            ctx.memory(|m| m.area_rect(id)).unwrap()
        };
        let short = rect(600.0);
        assert!(short.bottom() <= 600.0, "Window menu ends at {} in a 600 pt window", short.bottom());
        // With room to spare it shows whole, taller than the short window allowed.
        let tall = rect(3000.0);
        assert!(tall.height() > short.height() + 100.0, "{} vs {}", tall.height(), short.height());
        assert!(tall.bottom() < 3000.0);
    }
}
