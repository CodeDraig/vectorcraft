//! Execution of UI commands against application state.

use super::{
    Brightness, DockTab, Face, SaveMode, Value, VectorcraftApp, home_key, io, json, menu_dialog, next_zoom, recent_slot, saved_selection_name,
    saved_selection_slot, selected_image,
};

/// Canonical panel id for `window.panel`: a dock tab's or an icon panel's id or display label,
/// matched case-insensitively (`"Layers"`, `"swatches"`, `"Color Guide"`).
pub(crate) fn normalize_panel(input: &str) -> Option<&'static str> {
    let name = input.trim();
    crate::state::all_panels().find(|(id, label)| name.eq_ignore_ascii_case(id) || name.eq_ignore_ascii_case(label)).map(|(id, _)| id)
}

/// An optional `{key?: bool}` param: none when it is omitted or null (the commands then toggle).
pub(crate) fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// Handle a UI command. `None` = not a UI command (the engine handles it).
pub fn run_ui_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if id == "app.language" {
        let lang = p.get("lang").and_then(Value::as_str).unwrap_or("");
        let value = if lang.eq_ignore_ascii_case("auto") {
            "auto".to_string()
        } else if let Some(l) = crate::i18n::Lang::from_code(lang) {
            l.code().to_string()
        } else {
            let codes: Vec<&str> = std::iter::once("auto").chain(crate::i18n::Lang::all().map(|l| l.code())).collect();
            return Some(Err(format!("lang must be one of {}", codes.join(", "))));
        };
        return Some(app.run("prefs.set", json!({"key": "interfaceLanguage", "value": value})).map(|_| json!(value)));
    }
    if let Some(r) = crate::panels::character::intercept_text_command(app, id) {
        return Some(r);
    }
    if let Some(r) = crate::shortcut_editor::run_command(app, id, p)
        .or_else(|| crate::workspaces::run_command(app, id, p))
        .or_else(|| crate::floating::command(app, id, p))
    {
        return Some(r);
    }
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let flag = |b: &mut bool| {
        *b = !*b;
        Ok(json!(*b))
    };
    let r = match id {
        "file.newDialog" => {
            crate::dialogs::open_new_document(app);
            Ok(Value::Null)
        }
        "app.home" => {
            app.ui.home = Some(home_key(app));
            Ok(Value::Null)
        }
        "file.open" => match s("path") {
            Some(path) => io::open_path(app, &path),
            None => io::open_dialog(app).map(|_| Value::Null),
        },
        // Saves write through the app (save panel, download); with no path known the panel and the
        // format's options dialog ask first.
        "file.save" => io::save(app, SaveMode::Save, p, true),
        "file.saveAs" => io::save(app, SaveMode::SaveAs, p, true),
        "file.saveCopy" => io::save(app, SaveMode::Copy, p, true),
        "file.saveAsTemplate" => io::save(app, SaveMode::Template, p, true),
        "document.exportSelection" => {
            io::save_command_output(app, id, "png", if p.is_object() { p.clone() } else { json!({}) }).map(|p| json!({"path": p}))
        }
        // Bytes sent by an agent go straight to the engine; a path or nothing opens it here.
        "file.newFromTemplate" if p.get("dataBase64").is_none() => io::new_from_template(app, s("path")),
        "file.revert" if p.get("confirmed").and_then(Value::as_bool) != Some(true) => io::ask_revert(app),
        "file.reveal" => io::reveal(app),
        id if id.starts_with("file.openRecent") => match recent_slot(app, id).cloned() {
            Some(path) => io::open_path(app, &path),
            None => Err("no such recent file".into()),
        },
        "type.findFont" => {
            crate::find_font::open(app);
            Ok(Value::Null)
        }
        "file.clearRecent" => {
            app.ui.recent_files.clear();
            Ok(Value::Null)
        }
        "file.recentFiles" => Ok(json!(io::recent_files(app))),
        "file.place" => crate::place::run(app, p),
        "file.place.queue" => crate::place::queue(app, p),
        "file.export.svg" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::svg_options::open(app, crate::dialogs::svg_options::Mode::Export, None);
            Ok(Value::Null)
        }
        "file.export.svg" => io::export(app, Some("svg"), s("path"), p),
        "file.exportForScreens" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::open_export_for_screens(app);
            Ok(Value::Null)
        }
        "file.exportForScreens" => io::export_for_screens(app, p.clone()),
        "file.export.png" if s("path").is_none() => io::target_path(app, None, "png").and_then(|path| {
            let f = vectorcraft_engine::cmd::fileio::format("png").ok_or("no PNG encoder")?;
            let mut params = if p.is_object() { p.clone() } else { json!({}) };
            params["format"] = json!("png");
            params["path"] = json!(path);
            crate::dialogs::open_raster_options(app, f, params);
            Ok(Value::Null)
        }),
        "file.export.png" => io::export(app, Some("png"), s("path"), p),
        "file.documentSetup" => crate::dialogs::open_document_setup(app),
        "edit.preferences" => {
            crate::prefs_dialog::open(app, s("category").as_deref());
            Ok(Value::Null)
        }
        // Edit → Color Settings… / Assign Profile… (no params): show the colour-management panel.
        "edit.colorSettings" | "edit.assignProfile" if p.as_object().is_none_or(|o| o.is_empty()) => {
            app.ui.open_panel = Some("separations".into());
            app.ui.dock = true;
            app.session.execute(id, p).map_err(|e| e.to_string())
        }
        "view.outline" => flag(&mut app.ui.view.outline),
        "view.pixelPreview" => flag(&mut app.ui.view.pixel_preview),
        "view.trimView" => flag(&mut app.ui.view.trim_view),
        "view.cornerWidget" => flag(&mut app.ui.view.corner_widgets),
        // New View: the engine stores the current zoom, centre and rotation under the given name.
        "view.saved.new" if p.get("zoom").is_none() => {
            let Some(v) = app.view().copied() else { return Some(Err("no document".into())) };
            let mut params = if p.is_object() { p.clone() } else { json!({}) };
            params["center"] = json!([v.center.x, v.center.y]);
            params["zoom"] = json!(v.zoom);
            params["rotation"] = json!(v.rotation);
            app.session.execute(id, &params).map_err(|e| e.to_string())
        }
        id if id.starts_with("view.goto") => {
            let n: usize = id["view.goto".len()..].parse().unwrap_or(0);
            let saved = app.session.active().and_then(|d| n.checked_sub(1).and_then(|i| d.doc.views.get(i)).cloned());
            match (saved, app.view_mut()) {
                (Some(sv), Some(v)) => {
                    v.center = sv.center;
                    v.zoom = sv.zoom;
                    v.rotation = sv.rotation;
                    Ok(json!({ "name": sv.name }))
                }
                _ => Err("no such view".into()),
            }
        }
        id if saved_selection_slot(id).is_some() => match saved_selection_name(app, id) {
            Some(name) => app.run("select.recall", json!({ "name": name })),
            None => Err("no such saved selection".into()),
        },
        "view.snapToPixel" => flag(&mut app.ui.view.snap_to_pixel),
        "view.textThreads" => flag(&mut app.ui.view.text_threads),
        "type.hiddenCharacters" => flag(&mut app.ui.view.hidden_chars),
        "type.bold" => crate::panels::character::toggle_face(app, Face::Bold),
        "type.italic" => crate::panels::character::toggle_face(app, Face::Italic),
        "view.gradientAnnotator" => flag(&mut app.ui.view.gradient_annotator),
        "effect.last" => match app.last_effect.clone() {
            Some((e, params)) => {
                let label = vectorcraft_effects::effect_info(&e).map(|i| i.label.trim_end_matches('…').to_string()).unwrap_or_else(|| e.clone());
                let mut fields = params.as_object().cloned().unwrap_or_default();
                fields.insert("__effect".into(), json!(e));
                fields.insert("__label".into(), json!(label));
                fields.insert("preview".into(), json!(true));
                app.ui.dialog = Some(crate::state::Dialog { kind: "effect".into(), fields });
                Ok(Value::Null)
            }
            None => Err("no effect applied yet".into()),
        },
        id if id.starts_with("type.recentFont") => {
            let n: usize = id["type.recentFont".len()..].parse().unwrap_or(0);
            match n.checked_sub(1).and_then(|i| app.recent_fonts().get(i)).cloned() {
                Some(font) => app.run("text.setStyle", json!({ "font": font })),
                None => Err("no such recent font".into()),
            }
        }
        "view.edges" => flag(&mut app.ui.view.edges),
        "view.artboards" => flag(&mut app.ui.view.artboards),
        "view.rulers" => flag(&mut app.ui.view.rulers),
        "view.boundingBox" => flag(&mut app.ui.view.bounding_box),
        "view.guides" => {
            let r = flag(&mut app.ui.view.guides);
            // Hidden guides can't stay selected (selected, they are all that is).
            if !app.ui.view.guides && app.session.active().is_some_and(|d| !d.selection.guides.is_empty()) {
                // Deselecting a document that is open can't fail.
                let _ = app.run("select.none", json!({}));
            }
            r
        }
        "view.smartGuides" => flag(&mut app.ui.view.smart_guides),
        "view.grid" => flag(&mut app.ui.view.grid),
        "view.snapToGrid" => flag(&mut app.ui.view.snap_to_grid),
        "view.snapToPoint" => flag(&mut app.ui.view.snap_to_point),
        "view.zoomIn" | "view.zoomOut" => {
            let up = id == "view.zoomIn";
            // Selection & Anchor Display › Zoom to Selection: the selection comes to the middle.
            let focus = if app.session.prefs.zoom_to_selection { app.selection_box().map(|b| b.center()) } else { None };
            match app.view_mut() {
                Some(v) => {
                    v.zoom = next_zoom(v.zoom, up);
                    if let Some(c) = focus {
                        v.center = c;
                    }
                    Ok(json!({"zoom": v.zoom * 100.0}))
                }
                None => Err("no document".into()),
            }
        }
        "view.setZoom" => {
            let z = p.get("zoom").and_then(Value::as_f64).unwrap_or(100.0) / 100.0;
            let center =
                p.get("center").and_then(Value::as_array).and_then(|a| Some(vectorcraft_geom::Point::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
            match app.view_mut() {
                Some(v) => {
                    v.zoom = z.clamp(0.0313, 640.0);
                    v.fitted = true;
                    if let Some(c) = center {
                        v.center = c;
                    }
                    Ok(Value::Null)
                }
                None => Err("no document".into()),
            }
        }
        "view.fitArtboard" | "view.fitAll" | "view.actualSize" => {
            crate::canvas::fit(app, id);
            Ok(Value::Null)
        }
        "view.goToArtboard" => go_to_artboard(app, p),
        "view.presentation" => {
            app.ui.screen_mode = if app.ui.screen_mode == 3 { 0 } else { 3 };
            Ok(json!(app.ui.screen_mode))
        }
        "view.screenMode" => {
            app.ui.screen_mode = match p.get("mode").and_then(Value::as_u64) {
                Some(m) => m.min(3) as u8,
                // F cycles the three screen modes; from Presentation Mode (not in the cycle) it
                // goes back to Normal.
                None if app.ui.screen_mode >= 2 => 0,
                None => app.ui.screen_mode + 1,
            };
            Ok(json!(app.ui.screen_mode))
        }
        "view.rotateReset" => {
            if let Some(v) = app.view_mut() {
                v.rotation = 0.0;
            }
            Ok(Value::Null)
        }
        "window.control" => flag(&mut app.ui.control_bar),
        "window.toolbar" => flag(&mut app.ui.toolbar),
        "window.toolbarColumns" => opt_bool(p, "double").map(|on| {
            let on = on.unwrap_or(!app.ui.toolbar_double);
            app.ui.toolbar_double = on;
            json!(on)
        }),
        "window.toolbarAdvanced" => flag(&mut app.ui.toolbar_advanced),
        "window.floatTools" => {
            let floating = match opt_bool(p, "floating") {
                Ok(on) => on,
                Err(e) => return Some(Err(e)),
            };
            let Some(tool) = s("tool") else { return Some(Err("tool (a tool id) is required".into())) };
            crate::toolbar::float_group(app, &tool, floating).map(Value::Bool)
        }
        "window.taskBar" => flag(&mut app.ui.task_bar),
        "window.taskBar.pin" => {
            let place = &mut app.ui.task_bar_place;
            opt_bool(p, "pinned").map(|pinned| {
                let pinned = pinned.unwrap_or(!place.pinned);
                // Pinning holds the bar where it shows; unpinning lets it follow the selection from
                // there (the bar turns its pinned spot into an offset when it is next drawn).
                if pinned && !place.pinned {
                    place.pin_at = place.shown_at;
                }
                place.pinned = pinned;
                json!(pinned)
            })
        }
        "window.taskBar.reset" => {
            app.ui.task_bar_place = Default::default();
            Ok(Value::Null)
        }
        "window.dock" => {
            let on = !(app.ui.dock && app.ui.toolbar);
            app.ui.dock = on;
            app.ui.toolbar = on;
            app.ui.control_bar = on;
            Ok(json!(on))
        }
        "window.collapseDock" => opt_bool(p, "collapsed").map(|collapsed| {
            let collapsed = collapsed.unwrap_or(!app.ui.dock_collapsed);
            crate::dock::set_collapsed(app, collapsed);
            json!(collapsed)
        }),
        "window.panel" => {
            let raw = s("panel").unwrap_or_default();
            // Canonical id (case-insensitive; display labels work too), then the
            // dock tab it names, if any.
            let canonical = normalize_panel(&raw);
            // A floating panel shows its tab in its group.
            if let Some(p) = canonical
                && let Some(g) = crate::floating::group_of(&app.ui, p).and_then(|i| app.ui.floating_panels.get_mut(i))
            {
                g.active = g.panels.iter().position(|q| q == p).unwrap_or(g.active);
                let out = json!({ "floating": g.panels });
                app.ui.dock = true;
                return Some(Ok(out));
            }
            let tab = canonical.and_then(DockTab::from_id);
            match (canonical, tab) {
                // A collapsed dock pops the panel out of its icon, like the icon panels.
                (Some(p), Some(tab)) if app.ui.dock_collapsed => {
                    app.ui.dock_tab = tab;
                    app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(p) { None } else { Some(p.to_string()) };
                    app.ui.dock = true;
                    Ok(json!({"open": app.ui.open_panel}))
                }
                (Some(_), Some(tab)) => {
                    app.ui.dock_tab = tab;
                    Ok(Value::Null)
                }
                (Some(p), None) => {
                    app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(p) { None } else { Some(p.to_string()) };
                    app.ui.dock = true;
                    Ok(json!({"open": app.ui.open_panel}))
                }
                (None, _) => Err(format!("unknown panel `{raw}`")),
            }
        }
        "window.brightness" => match s("brightness").as_deref().and_then(Brightness::parse) {
            Some(b) => {
                app.ui.brightness = b;
                app.session.prefs.ui_brightness = b.id().into();
                app.canvas.key = None;
                Ok(json!(b.id()))
            }
            None => Err("brightness must be dark|mediumDark|mediumLight|light".into()),
        },
        "window.newWindow" => Err("multiple windows land with M11.5".into()),
        "tool.select" => match s("tool") {
            Some(t) if vectorcraft_tools::tool_info(&t).is_some() => {
                app.select_tool(&t);
                Ok(json!({"tool": app.session.tool_id()}))
            }
            Some(t) => Err(format!("unknown tool `{t}`")),
            None => Err("missing `tool`".into()),
        },
        "tool.setOption" => app.session.set_tool_option_cmd(p),
        "effect.dialog" => crate::dialogs::open_effect_dialog(app, p),
        "ui.recolorDialog" => crate::dialogs::recolor::open(app, p),
        "ui.cropImage" if !selected_image(app, |_| true) => Err("select an image".into()),
        "ui.cropImage" => {
            app.select_tool(vectorcraft_tools::cropimage::ID);
            Ok(json!({"tool": app.session.tool_id()}))
        }
        "ui.paramDialog" => {
            let cmd = s("command").unwrap_or_default();
            let mut fields = p.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
            fields.insert("__command".into(), json!(cmd));
            fields.insert("__label".into(), json!(s("label").unwrap_or(cmd.clone())));
            app.ui.dialog = Some(crate::state::Dialog { kind: "command".into(), fields });
            Ok(Value::Null)
        }
        "effect.applyLast" => match app.last_effect.clone() {
            Some((e, params)) => app.run("effect.apply", json!({"effect": e, "params": params})),
            None => Err("no effect applied yet".into()),
        },
        "file.export.pdf" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::open_save_pdf(app, &json!({})),
        "file.export.pdf" => io::export_pdf(app, p.clone()),
        "help.about" => {
            app.ui.about = true;
            Ok(Value::Null)
        }
        "help.commandPalette" => {
            app.ui.palette_open = !app.ui.palette_open;
            app.ui.palette_query.clear();
            Ok(Value::Null)
        }
        // Closing asks Save / Don't Save / Cancel for modified documents first (`unsaved`).
        "file.close" => {
            let i = p.get("index").and_then(Value::as_u64).map(|i| i as usize).or(app.session.active_index())?;
            crate::unsaved::close(app, i)
        }
        "file.closeAll" => crate::unsaved::close_all(app, "closeAll"),
        "app.quit" => crate::unsaved::close_all(app, "quit"),
        "ui.swatchOptions" => match s("name") {
            Some(name) => crate::dialogs::swatch_options::open(app, &name),
            None => Err("missing `name`".into()),
        },
        "ui.newSwatch" => crate::dialogs::new_swatch::open(app, p.get("spot").and_then(Value::as_bool).unwrap_or(false), s("group").as_deref()),
        "ui.newColorGroup" => {
            let names = p.get("swatches").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::new_color_group::open(app, names.unwrap_or_default())
        }
        "ui.colorPicker" => crate::dialogs::open_color_picker(app, p),
        "ui.graphicStyleOptions" => crate::dialogs::graphic_style_options::open(app, s("name").as_deref()),
        "tool.options" => crate::toolbar::open_options(app, &s("tool").unwrap_or_default()),
        "ui.colorGuideOptions" => {
            crate::dialogs::color_guide_options::open(app);
            Ok(Value::Null)
        }
        "ui.colorBalanceDialog" => {
            crate::dialogs::color_balance::open(app);
            Ok(Value::Null)
        }
        "ui.saturateDialog" => {
            crate::dialogs::saturate::open(app);
            Ok(Value::Null)
        }
        "window.swatchLibrary" => crate::panels::swatches::open_library(app, p),
        "window.swatchLibrary.other" => crate::panels::swatches::other_library(app, s("path")),
        "ui.saveSwatchLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_swatch_library::open(app, names.unwrap_or_default())
        }
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => match crate::panels::swatches::user_library(app, id) {
            Some(l) => crate::panels::swatches::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        "ui.mergeGraphicStyles" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::graphic_style_options::open_merge(app, names.unwrap_or_default())
        }
        "ui.tileEdgeColor" => crate::dialogs::tile_edge_color::open(app),
        "ui.layerOptions" => crate::dialogs::layer_options::open(app, p),
        "ui.newLayer" => crate::dialogs::layer_options::open_new(app, p),
        "ui.layersPanelOptions" => crate::dialogs::layers_panel_options::open(app),
        "ui.layersExpand" => crate::panels::layers::expand(app, p),
        "ui.flattenTransparencyDialog" => {
            crate::dialogs::flatten::open(app);
            Ok(Value::Null)
        }
        "ui.flattenerPresetsDialog" => {
            crate::dialogs::flattener_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.flattenerPreview" => crate::panels::flattener_preview::command(app, p),
        "window.graphicStyleLibrary" => crate::panels::graphic_styles::open_library(app, p),
        "window.graphicStyleLibrary.other" => crate::panels::graphic_styles::other_library(app, s("path")),
        "ui.saveGraphicStyleLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_style_library::open(app, names.unwrap_or_default())
        }
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => match crate::panels::graphic_styles::user_library(app, id) {
            Some(l) => crate::panels::graphic_styles::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        "ui.expandDialog" => crate::dialogs::expand::open(app),
        "attributes.openUrl" => crate::panels::attributes::open_url(app, p),
        "ui.spotColors" => crate::dialogs::spot_colors::open(app),
        "ui.menuDialog" => match s("command").as_deref() {
            Some(c) if crate::dialogs::envelope::opens(c) => crate::dialogs::envelope::open(app, c),
            c => match c.and_then(menu_dialog) {
                Some((kind, fields)) => {
                    app.ui.dialog = Some(crate::state::Dialog::new(kind, fields));
                    Ok(Value::Null)
                }
                None => Err("`command` must be a command whose menu item opens a dialog (see ui.menuDialog)".into()),
            },
        },
        "ui.widthPointEdit" => crate::dialogs::width_point::open(app, p),
        "ui.corners" => crate::dialogs::corners::open(app, p),
        "ui.colorGuideLimit" => crate::panels::color_guide::set_limit(app, p),
        "ui.savePdfDialog" => crate::dialogs::open_save_pdf(app, p),
        "file.exportAs" if s("path").is_none() => {
            crate::dialogs::open_export_as(app, s("format").as_deref(), p);
            Ok(Value::Null)
        }
        "file.exportAs" => io::export(app, s("format").as_deref(), s("path"), p),
        "ui.fileInfoDialog" => crate::dialogs::file_info::open(app),
        "ui.rasterEffectsSettingsDialog" => crate::dialogs::raster_effects::open(app),
        "ui.pdfPresetsDialog" => {
            crate::dialogs::pdf_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.pdfPresetDialog" => crate::dialogs::open_pdf_preset(app, p),
        // Save for Office Documents…: its dialog; with options, pick the file (or write `path`).
        "document.exportForOffice" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::office_export::open(app),
        "document.exportForOffice" => io::save_command_output(app, id, "png", p.clone()).map(|path| json!({ "path": path })),
        "ui.dxfOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.dxfOptionsDialog" => {
            crate::dialogs::dxf_options::open(app, p);
            Ok(Value::Null)
        }
        "links.editOriginal" => crate::panels::links::open_file(app, p, false),
        "links.reveal" => crate::panels::links::open_file(app, p, true),
        "ui.placementOptionsDialog" => crate::dialogs::placement_options::open(app, p),
        "ui.packageDialog" => crate::dialogs::package::open(app),
        "file.showPackage" => crate::dialogs::package::show_package(app, p),
        "docInfo.save" => crate::panels::doc_info::save_report(app, p),
        // Slice Options… and Divide Slices…: their dialogs, on the selected slices.
        "object.slice.options" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::slices::open_options(app),
        "object.slice.divide" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::slices::open_divide(app),
        "ui.epsOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.epsOptionsDialog" => {
            crate::dialogs::eps_options::open(app, p);
            Ok(Value::Null)
        }
        // Save for Web: its dialog; with settings, write the files.
        "file.saveForWeb" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::save_for_web::open(app),
        "file.saveForWeb" => crate::dialogs::save_for_web::save(app, p.clone()),
        "file.saveForWeb.browser" => crate::dialogs::save_for_web::browser_preview(app, p),
        // File → Export Selection…: the selection becomes assets, shown checked on Export for
        // Screens' Assets tab.
        "file.exportSelection" => app.run("assets.add", json!({ "multiple": p.get("multiple").and_then(Value::as_bool).unwrap_or(true) })).map(|r| {
            let ids: Vec<u64> = r["assets"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
            crate::dialogs::open_export_for_screens_assets(app, Some(&ids));
            json!({ "assets": ids })
        }),
        "css.copy" => crate::panels::css_properties::copy(app, p),
        "css.exportFile" => crate::panels::css_properties::export_file(app, p),
        // File → Print…: its dialog; with params, print (or save the job as a PDF) without it.
        "file.print" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::print::open(app),
        "file.print" => crate::print::run(app, p),
        "print.printers" => Ok(crate::print::printers(app)),
        "print.printerSetup" => {
            let printer = s("printer").filter(|n| !n.is_empty());
            match app.services.print.as_mut() {
                Some(service) => service.setup(printer.as_deref()).map(|_| Value::Null),
                None => Err("printing isn't available here".into()),
            }
        }
        "ui.printPresetsDialog" => {
            crate::dialogs::print_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.printPresetDialog" => crate::dialogs::print::open_preset(app, p),
        "plugin.dialog" => crate::dialogs::plugin::open(app, p),
        "ui.installPlugin" => io::install_plugin(app, s("path")),
        "ui.perspectiveGridDialog" => crate::dialogs::perspective_grid::open(app),
        "ui.perspectivePresetsDialog" => {
            crate::dialogs::perspective_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.savePerspectivePreset" => crate::dialogs::perspective_grid::open_save(app),
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => match crate::dialogs::perspective_presets::slot_preset(app, id) {
            Some(name) => app.run("perspective.grid.preset", json!({ "name": name })),
            None => Err("no such perspective grid preset".into()),
        },
        "ui.blendOptions" => crate::dialogs::blend_options::open(app),
        "ui.perspectivePlane" => crate::dialogs::perspective_plane::open(app, p),
        _ => return None,
    };
    Some(r)
}

/// `view.goToArtboard`: make artboard `index` (a number, or first / previous / next / last from
/// the current one) the navigator's, and fit it in the window.
fn go_to_artboard(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let n = app.session.active().map(|d| d.doc.artboards.len()).ok_or("no document open")?;
    let last = n.checked_sub(1).ok_or("the document has no artboards")?;
    let current = app.view().map_or(0, |v| v.artboard).min(last);
    let to = match p.get("index") {
        Some(Value::String(s)) => match s.as_str() {
            "first" => 0,
            "previous" => current.saturating_sub(1),
            "next" => current + 1,
            "last" => last,
            other => return Err(format!("index must be a number or first, previous, next or last, not `{other}`")),
        },
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("index must be a number or first, previous, next or last")?,
        None => return Err("missing index".into()),
    }
    .min(last);
    if let Some(v) = app.view_mut() {
        v.artboard = to;
    }
    crate::canvas::fit(app, "view.fitArtboard");
    Ok(json!({ "index": to }))
}
