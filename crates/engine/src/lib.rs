//! The VectorCraft engine façade.
//!
//! Every user-visible action is a command with a stable id (`object.group`, `select.same.fillColor`,
//! `shape.rectangle`…) and JSON parameters. The egui UI, the CLI, the control channel and the MCP
//! server all go through [`Session::execute`]. Tools (pointer gestures) are hosted here too and
//! reduce to commands, so every gesture is journaled and replayable.
#![forbid(unsafe_code)]

pub mod cmd;
mod doc_state;
mod documents;
pub mod guard;
pub mod inspect;
mod prefs;
mod tooling;
pub mod units;

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_color::{Color, GradientPaint, Paint};
#[cfg(test)]
use vectorcraft_doc::NodeKind;
use vectorcraft_doc::{Document, NodeId, Selection};
#[cfg(test)]
use vectorcraft_geom::Affine;
use vectorcraft_tools::{PaintDefaults, Tool};

pub use cmd::EyedropperOptions;
pub use cmd::clipboard::Clipboard;
pub use cmd::distortcmds::perspective_click;
pub use cmd::rasterfx::{export_pdf, flatten_raster_effects};
pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use doc_state::{DocState, History, HistoryEntry, Interaction, OpenRows, UndoGroup};
pub use prefs::Prefs;
pub use tooling::{UiRequest, ViewInfo};
pub use vectorcraft_doc as doc;
pub use vectorcraft_tools as tools;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active document")]
    NoDocument,
    #[error("no such object {0}")]
    NoNode(NodeId),
    #[error("{0}")]
    Other(String),
    /// A bug: the command panicked. The document was rolled back to its state before the command.
    #[error("internal error in `{cmd}`: {msg} (the document was left as it was before; please report this bug)")]
    Internal { cmd: String, msg: String },
}

impl From<vectorcraft_doc::DocError> for EngineError {
    fn from(e: vectorcraft_doc::DocError) -> Self {
        EngineError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Coordinates beyond this (points) are rejected: ~1,400 m, far past Illustrator's large canvas.
pub const MAX_COORD: f64 = 4.0e6;

/// `(command, param)` pairs the journal records only so a replay reproduces the original run
/// (dates from the clock: `cmd::clock_date`); an action leaves them out
/// ([`Session::journal_for_action`]).
const REPLAY_ONLY: [(&str, &str); 3] = [("file.new", "created"), ("document.save", "modified"), ("file.saveAs", "modified")];

/// `p`, the params of command `id`, without its [`REPLAY_ONLY`] values (a batch: its steps').
fn strip_replay_only(id: &str, p: &mut Value) {
    let strip = |id: &str, p: &mut Value| {
        if let Value::Object(m) = p {
            m.retain(|k, _| !REPLAY_ONLY.iter().any(|&(c, key)| c == id && key == k));
        }
    };
    strip(id, p);
    // Batches don't nest: one level of steps.
    if id == "command.batch"
        && let Some(Value::Array(steps)) = p.get_mut("commands")
    {
        for step in steps {
            let id = step.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(p) = step.get_mut("params") {
                strip(&id, p);
            }
        }
    }
}

/// Cheap sanity check after an edit: artboards and the objects just touched (the selection) must
/// have finite, in-range geometry, so saved files always reload and renderers never see NaN/∞.
fn doc_sane(d: &Document, sel: &Selection) -> bool {
    let ok = |r: vectorcraft_geom::Rect| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD);
    d.artboards.iter().all(|a| ok(a.rect))
        && d.guides.iter().all(|g| g.pos.is_finite() && g.pos.abs() <= MAX_COORD)
        && sel.objects.iter().all(|id| d.node(*id).and_then(|n| n.geometric_bounds()).is_none_or(ok))
}

/// Where new art goes (Illustrator's drawing modes, Shift+D cycles).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawMode {
    #[default]
    Normal,
    Behind,
    Inside,
}

pub struct Session {
    docs: Vec<DocState>,
    active: Option<usize>,
    pub prefs: Prefs,
    /// Fill/stroke for new art (the toolbar proxy).
    pub paint: PaintDefaults,
    /// Which proxy is in front (true = Fill, false = Stroke) — the X key toggles.
    pub fill_active: bool,
    /// Internal clipboard: the copied objects and the document resources they use.
    pub clipboard: Clipboard,
    /// Executed commands (for actions and debugging).
    pub journal: Vec<(String, Value)>,
    pub(crate) tool: Box<dyn Tool>,
    /// While Cmd lends `tool` (a selection tool) for a drag: the tool it was lent to, which comes
    /// back as it was at the release ([`Session::pointer`]).
    pub(crate) lender: Option<Box<dyn Tool>>,
    /// The selection tool chosen last (Selection, Direct Selection or Group Selection): the one Cmd
    /// lends the other tools. None until one is chosen.
    pub(crate) last_selection_tool: Option<&'static str>,
    pub(crate) last_view: ViewInfo,
    depth: u32,
    /// Set when the active tool panicked (see [`guard`]); reported by the next tool event.
    tool_panic: Option<EngineError>,
    /// Draw Normal / Behind / Inside (toolbar drawing modes).
    pub draw_mode: DrawMode,
    /// The path new art is drawn inside (Draw Inside).
    pub draw_inside: Option<NodeId>,
    untitled_counter: u32,
    /// Session-level state of the menu commands (saved selections, guide lock).
    pub(crate) menu: cmd::menucmds::MenuState,
    /// The selected gradient stop (`gradient.selectStop`) and whose gradient it was selected on,
    /// shared by the Gradient tool's annotator, the Gradient and Color panels and agents: read it
    /// with [`Session::selected_stop`].
    pub(crate) gradient_stop: Option<(usize, cmd::gradient::StopOwner)>,
    /// The Appearance panel's active fill/stroke row (`appearance.setActiveItem`); not saved. Read
    /// it through [`Session::appearance_item`], which drops it once the selection changes.
    pub(crate) active_appearance_item: Option<cmd::appearance::ActiveItem>,
    /// The last solid colour applied (the toolbar's Color button; `,` applies it again).
    pub last_solid: Color,
    /// The last gradient applied (the toolbar's Gradient button; `.` applies it again).
    pub last_gradient: GradientPaint,
    /// Recently applied solid colours, newest first (the Recent Colors rows), fed by every paint
    /// command whichever frontend runs it.
    pub recent_colors: Vec<Color>,
    /// A paint applied by a live preview: remembered when the interaction commits.
    pub(crate) pending_paint: Option<Paint>,
    /// User Defined and loaded swatch libraries (Window → Swatch Libraries); not saved.
    pub swatch_libraries: cmd::swatchlib::Libraries,
    /// The selected freeform gradient point (`paint.freeform.selectPoint`) and whose gradient it
    /// was selected on: read it with [`Session::selected_freeform_point`].
    pub(crate) freeform_point: Option<(usize, cmd::gradient::StopOwner)>,
    /// User Defined and loaded graphic style libraries (Window → Graphic Style Libraries); not saved.
    pub style_libraries: cmd::stylelib::Libraries,
    /// The Libraries panel's libraries of graphics, colours and text styles (`library.*`).
    pub libraries: cmd::library::Libraries,
    /// URLs recently given in the Attributes panel (`attributes.set {url}`), newest first; not saved.
    pub recent_urls: Vec<String>,
    /// The language the UI is drawn in (a language code, never `auto`), set by the UI each frame;
    /// `None` without one (headless), where an explicit `interfaceLanguage` preference counts.
    /// Japanese gives new type the Japanese defaults ([`Session::japanese_interface`]).
    pub ui_language: Option<String>,
    /// Parameters the running top-level command resolved from the preferences or the clock, added
    /// to its journal entry so a replay does the same ([`Session::note_journal`]).
    journal_note: serde_json::Map<String, Value>,
    /// The depth of the command whose values [`Session::note_journal`] keeps: 1 (the top-level
    /// command), or a step's while a batch runs it ([`Session::execute_step`]).
    note_depth: u32,
    /// While `command.batch` runs: the documents its steps closed or replaced (Revert), which an
    /// error brings back; `None` otherwise.
    pub(crate) batch_stash: Option<Vec<DocState>>,
    /// Where Data Recovery keeps its copies ([`cmd::recovery`]).
    pub recovery: cmd::recovery::Recovery,
    /// While the active tool's actions run: their commands aren't "a command from outside the
    /// tool" ([`Session::after_command`]).
    pub(crate) in_tool_actions: bool,
    /// Envelope Options with no envelope selected: the options and fidelity new envelopes get
    /// (`None`: the reference app's defaults, fidelity 50); not saved.
    pub(crate) envelope_defaults: Option<(vectorcraft_doc::live::EnvelopeOptions, f64)>,
    /// The Liquify stroke the last live preview applied, which the next sample of the drag goes on
    /// from ([`cmd::distortcmds::LiquifyStroke`]).
    pub(crate) liquify_stroke: Option<Box<cmd::distortcmds::LiquifyStroke>>,
    /// A press on the Plane Switching Widget is under way: its drag and release are the widget's.
    pub(crate) plane_widget_press: bool,
    /// A guide being dragged out of a ruler ([`Session::ruler_guide`]).
    pub(crate) ruler_guide: Option<vectorcraft_tools::rulerguide::NewGuide>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        // Placed documents are read by the native format's loader.
        vectorcraft_doc::placed_document::set_loader(cmd::place::document::read_document);
        vectorcraft_doc::placed_document::set_file_reader(cmd::place::document::read_file_again);
        Self {
            docs: vec![],
            active: None,
            prefs: Prefs::default(),
            paint: PaintDefaults::default(),
            fill_active: true,
            clipboard: Clipboard::default(),
            journal: vec![],
            tool: vectorcraft_tools::create("selection"),
            lender: None,
            last_selection_tool: None,
            last_view: ViewInfo::default(),
            depth: 0,
            tool_panic: None,
            draw_mode: DrawMode::Normal,
            draw_inside: None,
            untitled_counter: 0,
            menu: Default::default(),
            gradient_stop: None,
            active_appearance_item: None,
            last_solid: Color::WHITE,
            last_gradient: GradientPaint::new(Default::default()),
            recent_colors: vec![],
            pending_paint: None,
            swatch_libraries: Default::default(),
            freeform_point: None,
            style_libraries: Default::default(),
            libraries: Default::default(),
            recent_urls: vec![],
            ui_language: None,
            journal_note: Default::default(),
            note_depth: 1,
            batch_stash: None,
            recovery: Default::default(),
            in_tool_actions: false,
            envelope_defaults: None,
            liquify_stroke: None,
            plane_widget_press: false,
            ruler_guide: None,
        }
    }

    /// Execute a command by id. This is THE entry point for every frontend.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        // Clear's menu needs a selection; an agent supplying explicit targets only needs a
        // document. The command validates those targets before making any edit.
        let enabled = if id == "edit.clear" && params.get("ids").is_some() { cmd::has_doc(self) } else { (spec.enabled)(self) };
        if let Err(why) = enabled {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        // Only top-level commands are journaled (commands that call other commands would otherwise
        // be recorded twice and replay differently).
        if self.depth == 0 {
            self.journal_note.clear();
            self.note_depth = 1;
            self.batch_stash = None;
        }
        let r = if self.depth == 0 { self.run_guarded(id, |s| (spec.run)(s, params)) } else { self.run_nested(|s| (spec.run)(s, params)) };
        let r = r?;
        if self.depth == 0 {
            self.inherit_new_art();
        }
        if spec.journal && self.depth == 0 && self.active().is_none_or(|d| d.interaction.is_none()) {
            let p = self.noted(params);
            self.journal.push((id.to_string(), p));
        }
        if self.depth == 0 {
            self.after_command();
        }
        Ok(r)
    }

    /// Is the interface in Japanese? Then new type starts with em box top-to-top leading and em
    /// box centre character alignment (#432).
    pub fn japanese_interface(&self) -> bool {
        self.ui_language.as_deref().unwrap_or(&self.prefs.interface_language).eq_ignore_ascii_case("ja")
    }

    /// Record `key: value` in the running top-level command's journal entry (or its interaction's
    /// preview, or its step in a batch) unless its params give `key`: a value it resolved from the
    /// preferences or the clock.
    pub fn note_journal(&mut self, key: &str, value: Value) {
        if self.depth == self.note_depth {
            self.journal_note.insert(key.to_string(), value);
        }
    }

    /// Run `id` as a step of the running command, which journals its steps (`command.batch`):
    /// → the step's result and its params with what it noted ([`Session::note_journal`]), for the
    /// step in the running command's journal entry.
    pub(crate) fn execute_step(&mut self, id: &str, params: &Value) -> Result<(Value, Value)> {
        let outer = (std::mem::take(&mut self.journal_note), self.note_depth);
        self.note_depth = self.depth + 1;
        let r = self.execute(id, params);
        let noted = self.noted(params);
        (self.journal_note, self.note_depth) = outer;
        Ok((r?, noted))
    }

    /// The journal from entry `start` on, as an action records it: without the params that only
    /// pin a replay to the original run ([`REPLAY_ONLY`]), so playing the action later acts now.
    pub fn journal_for_action(&self, start: usize) -> Vec<(String, Value)> {
        self.journal
            .iter()
            .skip(start)
            .cloned()
            .map(|(id, mut p)| {
                strip_replay_only(&id, &mut p);
                (id, p)
            })
            .collect()
    }

    /// `params` with the noted values added (see [`Session::note_journal`]).
    fn noted(&mut self, params: &Value) -> Value {
        let mut note = std::mem::take(&mut self.journal_note);
        match params {
            // The params' own values win.
            Value::Object(m) if !note.is_empty() => {
                note.extend(m.clone());
                Value::Object(note)
            }
            Value::Null if !note.is_empty() => Value::Object(note),
            _ => params.clone(),
        }
    }

    fn run_nested(&mut self, f: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    /// A top-level command: a panic (a bug) becomes [`EngineError::Internal`] and the active
    /// document goes back to how it was before the command, instead of crashing the frontend.
    fn run_guarded(&mut self, id: &str, f: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        let snapshot = self.active().map(|d| (d.uid, d.doc.clone(), d.selection.clone(), d.interaction.clone()));
        let active = self.active;
        match guard::catch_panic(|| self.run_nested(f)) {
            Ok(r) => r,
            Err(msg) => {
                self.depth = 0;
                self.journal_note.clear();
                if let Some((uid, doc, selection, interaction)) = snapshot
                    && let Some(st) = self.docs.iter_mut().find(|d| d.uid == uid)
                {
                    st.doc = doc;
                    st.selection = selection;
                    st.interaction = interaction;
                    st.revision += 1;
                }
                if active.is_some_and(|i| i < self.docs.len()) {
                    self.active = active;
                }
                Err(EngineError::Internal { cmd: id.to_string(), msg })
            }
        }
    }

    /// Run `f` on a mutable copy of the active document. Outside an interaction this records one
    /// undo step labelled `label`; inside one it just mutates (the interaction commits later).
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>) -> Result<T> {
        let st = self.doc_mut()?;
        let before = st.doc.clone();
        let before_sel = st.selection.clone();
        let doc = Arc::make_mut(&mut st.doc);
        let result = match f(doc, &mut st.selection) {
            Ok(v) => match cmd::shaper::refresh(&before, Arc::make_mut(&mut st.doc)) {
                Err(e) => Err(e),
                Ok(()) => {
                    // Inline graphics in text follow their symbols.
                    cmd::inline::refresh(&before, Arc::make_mut(&mut st.doc));
                    // Text Wrap: area type follows its wrap objects; then threads re-flow.
                    cmd::textwrap::refresh(Arc::make_mut(&mut st.doc));
                    // Opacity-mask editing: the mask follows its art on the editing layer.
                    if st.doc.mask_edit.is_some() {
                        cmd::maskedit::sync(Arc::make_mut(&mut st.doc));
                    }
                    // Threaded text re-flows when any of its frames changed.
                    if !st.doc.text_threads.is_empty() {
                        cmd::threads::reflow(&before, Arc::make_mut(&mut st.doc));
                    }
                    // Asset Export: assets let go of deleted art.
                    if !st.doc.assets.is_empty() {
                        Arc::make_mut(&mut st.doc).prune_assets();
                    }
                    if doc_sane(&st.doc, &st.selection) {
                        Ok(v)
                    } else {
                        Err(EngineError::Other("result would exceed the canvas (coordinates out of range)".into()))
                    }
                }
            },
            Err(e) => Err(e),
        };
        match result {
            Ok(v) => {
                st.selection.prune(&st.doc);
                st.revision += 1;
                if st.interaction.is_none() {
                    st.push_undo(HistoryEntry { label: label.to_string(), doc: before, selection: before_sel });
                }
                Ok(v)
            }
            Err(e) => {
                st.doc = before;
                st.selection = before_sel;
                Err(e)
            }
        }
    }

    /// Change only the selection (not an undo step).
    pub fn select(&mut self, f: impl FnOnce(&Document, &mut Selection)) -> Result<()> {
        self.active_appearance_item = None;
        let st = self.doc_mut()?;
        let before = st.selection.objects.clone();
        f(&st.doc, &mut st.selection);
        st.selection.prune(&st.doc);
        // Selecting art makes its layer (or sublayer) the current one, as in the Layers panel of
        // the reference app: new art then goes beside it.
        if st.selection.objects != before
            && let Some(layer) = st.selection.objects.last().and_then(|id| st.doc.layer_containing(*id)).filter(|l| st.doc.is_editable(*l))
            && st.active_layer != Some(layer)
        {
            st.active_layer = Some(layer);
            st.layer_rows.clear();
        }
        st.revision += 1;
        // Puppet Warp pins belong to the art they were placed on: another selection starts afresh.
        if st.doc.puppet.as_ref().is_some_and(|p| p.ids != st.selection.objects) {
            cmd::distortcmds::drop_puppet_pins(st);
        }
        Ok(())
    }

    // ---------- interactions (live drags) ----------

    pub fn begin_interaction(&mut self, label: &str) -> Result<()> {
        let st = self.doc_mut()?;
        if st.interaction.is_some() {
            return Ok(());
        }
        st.interaction = Some(Interaction {
            label: label.to_string(),
            doc: st.doc.clone(),
            selection: st.selection.clone(),
            preview: None,
            active_layer: st.active_layer,
            layer_rows: st.layer_rows.clone(),
            isolation: st.isolation,
            perspective_again: None,
        });
        Ok(())
    }

    /// Re-apply `cmd` on top of the interaction snapshot (replacing the previous preview).
    pub fn preview(&mut self, cmd: &str, params: &Value) -> Result<Value> {
        {
            let st = self.doc_mut()?;
            let Some(it) = &st.interaction else { return Err(EngineError::Other("no interaction in progress".into())) };
            st.doc = it.doc.clone();
            st.selection = it.selection.clone();
        }
        let rw = cmd::distortcmds::perspective_rewrite(self, cmd, params);
        let (cmd, params) = rw.as_ref().map_or((cmd, params), |(c, p)| (c.as_str(), p));
        let r = self.execute(cmd, params);
        let params = self.noted(params);
        let st = self.doc_mut()?;
        if let Some(it) = &mut st.interaction {
            it.preview = Some((cmd.to_string(), params));
        }
        r
    }

    pub fn commit_interaction(&mut self) -> Result<()> {
        self.remember_pending_paint();
        let st = self.doc_mut()?;
        let Some(mut it) = st.interaction.take() else { return Ok(()) };
        let Some(preview) = it.preview.take() else { return Ok(()) };
        let perspective_again = it.perspective_again.take();
        st.keep_interaction(it);
        if preview.0 == "object.transform" {
            let m = cmd::matrix_param(&preview.1, "matrix");
            let copy = preview.1.get("copy").and_then(Value::as_bool).unwrap_or(false);
            if let Some(m) = m {
                st.last_transform = Some((m, copy));
                st.last_perspective = None;
            }
        }
        if perspective_again.is_some() {
            st.last_perspective = perspective_again;
        }
        self.journal.push(preview);
        Ok(())
    }

    /// Remember the paint a live preview applied (as its interaction is kept).
    pub(crate) fn remember_pending_paint(&mut self) {
        if let Some(p) = self.pending_paint.take() {
            self.remember_paint_now(&p);
        }
    }

    pub fn cancel_interaction(&mut self) -> Result<()> {
        self.pending_paint = None;
        self.doc_mut()?.undo_interaction();
        Ok(())
    }

    pub fn in_interaction(&self) -> bool {
        self.active().is_some_and(|d| d.interaction.is_some())
    }

    // ---------- undo groups (scrubbed numeric fields) ----------

    /// Open an undo group in the active document: until [`Session::end_undo_group`], the edits
    /// made there (commands, committed interactions) are one undo step. A scrubbed numeric field
    /// applies each value it passes as its own command, as a typed value is applied; the drag is
    /// one step.
    pub fn begin_undo_group(&mut self) {
        let journal = self.journal.len();
        if let Some(st) = self.active_mut()
            && st.undo_group.is_none()
        {
            st.undo_group = Some(UndoGroup { first: None, journal });
        }
    }

    /// Close the open undo groups: their edits stay one undo step or, `cancel`led (Escape), are
    /// undone and dropped from the journal.
    pub fn end_undo_group(&mut self, cancel: bool) {
        let mut journal = None;
        for st in &mut self.docs {
            let Some(g) = st.undo_group.take() else { continue };
            // Only while the group's step is still the newest one.
            if cancel
                && let Some(first) = g.first
                && st.history.undo.last().is_some_and(|e| Arc::ptr_eq(&e.doc, &first))
                && let Some(e) = st.history.undo.pop()
            {
                st.doc = e.doc;
                st.selection = e.selection;
                st.revision += 1;
                journal = Some(g.journal);
            }
        }
        if let Some(len) = journal {
            self.journal.truncate(len);
        }
    }

    /// Commands with enablement (for menus, palette, MCP `command_list`).
    pub fn commands(&self) -> Vec<CommandInfo> {
        command_specs().iter().map(|c| c.info(self)).collect()
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_adjust;
#[cfg(test)]
mod tests_appearance;
#[cfg(test)]
mod tests_areafit;
#[cfg(test)]
mod tests_assets;
#[cfg(test)]
mod tests_attributes;
#[cfg(test)]
mod tests_bboxrotate;
#[cfg(test)]
mod tests_blendfidelity;
#[cfg(test)]
mod tests_blendopts;
#[cfg(test)]
mod tests_blendspine;
#[cfg(test)]
mod tests_brushsym;
#[cfg(test)]
mod tests_build;
#[cfg(test)]
mod tests_charstroke;
#[cfg(test)]
mod tests_clip;
#[cfg(test)]
mod tests_clipboard;
#[cfg(test)]
mod tests_clipflavours;
#[cfg(test)]
mod tests_clippaint;
#[cfg(test)]
mod tests_cmdsplit;
#[cfg(test)]
mod tests_cmykflatten;
#[cfg(test)]
mod tests_colorguide;
#[cfg(test)]
mod tests_colorguidelib;
#[cfg(test)]
mod tests_colormgmt;
#[cfg(test)]
mod tests_colorthemes;
#[cfg(test)]
mod tests_containers;
#[cfg(test)]
mod tests_css;
#[cfg(test)]
mod tests_cut;
#[cfg(test)]
mod tests_dashalign;
#[cfg(test)]
mod tests_distort;
#[cfg(test)]
mod tests_docsetup;
#[cfg(test)]
mod tests_draw2;
#[cfg(test)]
mod tests_editcolors;
#[cfg(test)]
mod tests_effectedit;
#[cfg(test)]
mod tests_emptytype;
#[cfg(test)]
mod tests_envelope;
#[cfg(test)]
mod tests_envelope_distort;
#[cfg(test)]
mod tests_envelope_edit;
#[cfg(test)]
mod tests_expand;
#[cfg(test)]
mod tests_eyedropper;
#[cfg(test)]
mod tests_file;
#[cfg(test)]
mod tests_fileinfo;
#[cfg(test)]
mod tests_flatpresets;
#[cfg(test)]
mod tests_flatpreview;
#[cfg(test)]
mod tests_flatten;
#[cfg(test)]
mod tests_focal;
#[cfg(test)]
mod tests_fontlist;
#[cfg(test)]
mod tests_freeform;
#[cfg(test)]
mod tests_gradient;
#[cfg(test)]
mod tests_gradpanel;
#[cfg(test)]
mod tests_halftone;
#[cfg(test)]
mod tests_inline;
#[cfg(test)]
mod tests_journal;
#[cfg(test)]
mod tests_knockout;
#[cfg(test)]
mod tests_labspots;
#[cfg(test)]
mod tests_layerclip;
#[cfg(test)]
mod tests_layers;
#[cfg(test)]
mod tests_library;
#[cfg(test)]
mod tests_linked_stops;
#[cfg(test)]
mod tests_links;
#[cfg(test)]
mod tests_linkspanel;
#[cfg(test)]
mod tests_liquify;
#[cfg(test)]
mod tests_live;
#[cfg(test)]
mod tests_livecorners;
#[cfg(test)]
mod tests_maskview;
#[cfg(test)]
mod tests_menucmds;
#[cfg(test)]
mod tests_nativefile;
#[cfg(test)]
mod tests_newart;
#[cfg(test)]
mod tests_newdoc;
#[cfg(test)]
mod tests_objexpand;
#[cfg(test)]
mod tests_opacitymask;
#[cfg(test)]
mod tests_outlinestroke;
#[cfg(test)]
mod tests_overprint;
#[cfg(test)]
mod tests_package;
#[cfg(test)]
mod tests_paintproxy;
#[cfg(test)]
mod tests_panelcmds;
#[cfg(test)]
mod tests_paragraphs;
#[cfg(test)]
mod tests_pathops;
#[cfg(test)]
mod tests_pathtype;
#[cfg(test)]
mod tests_pattern;
#[cfg(test)]
mod tests_pdffidelity;
#[cfg(test)]
mod tests_pdfmarks;
#[cfg(test)]
mod tests_pdfpresets;
#[cfg(test)]
mod tests_pdfraster;
#[cfg(test)]
mod tests_persp_planes;
#[cfg(test)]
mod tests_persp_select;
#[cfg(test)]
mod tests_persp_text;
#[cfg(test)]
mod tests_perspgrid;
#[cfg(test)]
mod tests_place;
#[cfg(test)]
mod tests_placed_document;
#[cfg(test)]
mod tests_plugins;
#[cfg(test)]
mod tests_prefs;
#[cfg(test)]
mod tests_previewbounds;
#[cfg(test)]
mod tests_print;
#[cfg(test)]
mod tests_printadvanced;
#[cfg(test)]
mod tests_printpresets;
#[cfg(test)]
mod tests_printpreview;
#[cfg(test)]
mod tests_printps;
#[cfg(test)]
mod tests_printtiling;
#[cfg(test)]
mod tests_proxyitems;
#[cfg(test)]
mod tests_puppetwarp;
#[cfg(test)]
mod tests_rasterfilters;
#[cfg(test)]
mod tests_rastersettings;
#[cfg(test)]
mod tests_recolor;
#[cfg(test)]
mod tests_recovery;
#[cfg(test)]
mod tests_registration;
#[cfg(test)]
mod tests_save;
#[cfg(test)]
mod tests_saveoptions;
#[cfg(test)]
mod tests_scalestrokes;
#[cfg(test)]
mod tests_shaper;
#[cfg(test)]
mod tests_slices;
#[cfg(test)]
mod tests_smartguides;
#[cfg(test)]
mod tests_strokegeom;
#[cfg(test)]
mod tests_strokegradient;
#[cfg(test)]
mod tests_strokereach;
#[cfg(test)]
mod tests_strokeux;
#[cfg(test)]
mod tests_stylelib;
#[cfg(test)]
mod tests_stylepanel;
#[cfg(test)]
mod tests_styles;
#[cfg(test)]
mod tests_swatchcmds;
#[cfg(test)]
mod tests_swatches;
#[cfg(test)]
mod tests_swatchlib;
#[cfg(test)]
mod tests_targeting;
#[cfg(test)]
mod tests_textcombos;
#[cfg(test)]
mod tests_textedit;
#[cfg(test)]
mod tests_textimport;
#[cfg(test)]
mod tests_tileedge;
#[cfg(test)]
mod tests_tints;
#[cfg(test)]
mod tests_toolsettings;
#[cfg(test)]
mod tests_transparencygrid;
#[cfg(test)]
mod tests_typearea;
#[cfg(test)]
mod tests_units;
#[cfg(test)]
mod tests_webexport;
#[cfg(test)]
mod tests_widthpoints;
#[cfg(test)]
mod tests_widthprofiles;
#[cfg(test)]
mod tests_widthtool;
#[cfg(test)]
mod tests_xform;
