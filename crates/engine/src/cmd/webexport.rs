//! File → Export → Save for Web (Legacy): an optimised web image — GIF, JPEG, PNG-8 or PNG-24 —
//! of an artboard or of the art, or one image per slice with an HTML page placing them, plus the
//! named settings the dialog and `webExport.presets.*` keep ([`WebSettings`], [`WebPreset`]).
//!
//! The pipeline: [`render`] draws the region once (straight RGBA at the output size); [`optimize`]
//! turns pixels into a file with the settings (palette reduction and the colour table's edits, web
//! snap, lossy, JPEG quality, metadata); [`decode`] reads a file back for the dialog's preview, so
//! what the preview shows and the size it reports are what Save writes. Slices are cut from the one
//! rendering, each optimised on its own.

mod settings;
pub use settings::{ColorTable, SliceScope, WebFormat, WebMetadata, WebOutput, WebPreset, WebSettings, builtin_presets};
use settings::{is_builtin, saved_index};
mod encoding;
pub use encoding::{Optimized, TableColor, decode, hex, optimize, preview_image};

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use vectorcraft_doc::slices::SliceArea;
use vectorcraft_doc::{CellAlign, CellVAlign, Document, NodeId, NodeKind, SliceKind};
use vectorcraft_geom::Rect;
use vectorcraft_render::AntiAlias;
use vectorcraft_render::encode::quantize::{self, Dither, Indexed, PaletteOptions, Reduction};
use vectorcraft_render::encode::web::{self, ColorSort};
use vectorcraft_render::encode::{gif, jpeg, png};

use super::*;

const C: &str = "document.exportForWeb";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "document.exportForWeb",
            "Save for Web",
            [],
            None,
            "{path?, preset?: a webExport.presets name (its settings, then these keys), format?: gif (default)|jpg|png8|png24, gif/png8: reduction?: perceptual|selective (default)|adaptive|web|blackWhite|gray, colors?: 128 (2–256), dither?: none|diffusion (default)|pattern|noise, ditherAmount?: 100, webSnap?: 0 (0–100 %: colours that close to a web-safe one become it), lossy?: 0 (0–100, gif), colorTable?: {locked?: [\"#rrggbb\"…] (kept when reducing), transparent?: [\"#rrggbb\"…] (become transparent), webShift?: [\"#rrggbb\"…] (become web-safe), sort?: none|hue|luminance|popularity} (colours as the preview's colors[].source), transparency?: true (gif/png8/png24), matte?: white (default)|black|none|\"#rrggbb\" (what edges blend into; jpg and opaque png24: the background), interlaced?: false (gif/png8/png24), jpg: quality?: 60 (0–100), progressive?: false, optimized?: true, embedProfile?: false (sRGB profile), width?: px | height?: px | percent?: 100 (the image size, proportional; width wins over height, height over percent), antiAlias?: none|art (default)|type, clipToArtboard?: true (the artboard; false: the art and the slices), artboard?: 0 (0-based), convertToSrgb?: true (PNG files say they are sRGB), metadata?: none|copyright (default)|contact|all (File Info written as PNG text, a GIF or JPEG comment), slices?: all (default)|selected|none (one image per slice: images/<slice name>.<ext>), output?: images (default)|html (an HTML page placing the image or slices, <stem>.html)} → {path, files: [path…], bytes (total), width, height}; no path → {files: [{name, dataBase64}], bytes, width, height} (names such as page.html, images/page_01.gif). A single image without HTML is written at path itself",
            has_doc,
            export
        ),
        cmd!(
            query "document.exportForWeb.preview",
            "Save for Web Preview",
            [],
            None,
            "{…the settings of document.exportForWeb (preset?, format?…), slice?: n (1-based, from slice.list; default the whole image), kbps?: 56.6 (connection speed for the download time), image?: false (also the optimised file as dataBase64)} what Save writes for the whole image (or one slice) → {format, width, height, bytes (exactly the file's size), seconds (to download at kbps), kbps, colors?: [{color: \"#rrggbb\", source: \"#rrggbb\" (the colour the reduction made, as colorTable lists it), transparent, locked, webShifted, webSafe}] (gif/png8: the colour table in file order), mappedToTransparent?: [\"#rrggbb\"…], slices: images an export writes, dataBase64?}",
            has_doc,
            preview
        ),
        cmd!(
            query "webExport.settings",
            "Save for Web Settings",
            [],
            None,
            "{reset?: false, preset?, …the settings of document.exportForWeb} remember settings for the Save for Web dialog (it opens on them): the keys given change the remembered ones, a preset replaces them, reset goes back to the defaults; {} reads them → {settings}",
            always,
            settings
        ),
        cmd!(
            query "webExport.presets.list",
            "Save for Web Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, settings}]} the built-in presets, then the saved ones; any name works as `preset` in document.exportForWeb",
            always,
            presets_list
        ),
        cmd!(
            query "webExport.presets.save",
            "Save Save for Web Preset",
            [],
            None,
            "{name, newName?: rename it, preset?: start from this preset (default: the saved preset `name`, else the defaults), settings?: {…}, …the settings of document.exportForWeb} create or change a saved preset (built-in ones can't change) → {name, settings, created}",
            always,
            presets_save
        ),
        cmd!(
            query "webExport.presets.delete",
            "Delete Save for Web Preset",
            [],
            None,
            "{name} delete a saved preset (built-in ones stay) → {deleted: name}",
            always,
            presets_delete
        ),
    ]
}

// ---------- the pipeline ----------

/// The region an export covers and its pixels per point.
pub fn region(doc: &Document, s: &WebSettings) -> std::result::Result<(Rect, f64), String> {
    let r = if s.clip_to_artboard {
        let i = s.artboard.unwrap_or(0);
        doc.artboards.get(i).map(|a| a.rect).ok_or_else(|| format!("no artboard {i} (0-based; the document has {})", doc.artboards.len()))?
    } else {
        let slices = if s.slices == SliceScope::None { vec![] } else { doc.slice_layout() };
        slices
            .iter()
            .fold(vectorcraft_render::encode::art_bounds(doc), |acc, a| vectorcraft_geom::union_opt(acc, Some(a.rect)))
            .ok_or("nothing to save: the document has no visible art")?
    };
    let scale = match (s.width, s.height, s.percent) {
        (Some(w), _, _) => f64::from(w) / r.width(),
        (None, Some(h), _) => f64::from(h) / r.height(),
        (None, None, Some(p)) => p / 100.0,
        _ => 1.0,
    };
    vectorcraft_render::raster_size(r, scale)?;
    Ok((r, scale))
}

/// A rendering of the region an export covers, before optimisation.
#[derive(Clone, Debug)]
pub struct WebRender {
    /// Straight RGBA, `width`×`height`.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub region: Rect,
    /// Pixels per point.
    pub scale: f64,
}

/// Draw what `s` exports of `doc` (template layers left out; on white when the document's
/// background is white).
pub fn render(doc: &Document, s: &WebSettings) -> std::result::Result<WebRender, String> {
    let doc = doc.without_edit_modes();
    let (region, scale) = region(&doc, s)?;
    let page = (doc.setup.background == vectorcraft_doc::Background::White).then_some([255; 4]);
    let opts = vectorcraft_render::RenderOptions { background: page, skip_templates: true, anti_alias: s.anti_alias, ..Default::default() };
    let img = vectorcraft_render::Renderer::new().render_region_with(&doc, region, scale, &opts);
    Ok(WebRender { rgba: img.to_straight(), width: img.width, height: img.height, region, scale })
}

/// Seconds to download `bytes` at `kbps` kilobits per second.
pub fn download_seconds(bytes: usize, kbps: f64) -> f64 {
    if kbps > 0.0 { bytes as f64 * 8.0 / (kbps * 1000.0) } else { 0.0 }
}

/// Connection speeds the dialog offers (kilobits per second).
pub const SPEEDS: [f64; 8] = [28.8, 56.6, 128.0, 256.0, 512.0, 1024.0, 2048.0, 8192.0];

// ---------- slices and the HTML page ----------

/// One slice as written: its area in image pixels and what its cell holds.
struct Cell<'a> {
    area: &'a SliceArea,
    /// Pixel rectangle `[x0, y0, x1, y1]` in the image.
    px: [u32; 4],
}

/// The slices of `doc` that get cells (`selected`: the selected slice ids for
/// [`SliceScope::Selected`]), in pixels of `r`. Empty when the document has none (or `s` says none).
fn cells<'a>(areas: &'a [SliceArea], selected: &[NodeId], s: &WebSettings, r: &WebRender) -> std::result::Result<Vec<Cell<'a>>, String> {
    if s.slices == SliceScope::None {
        return Ok(vec![]);
    }
    let edge = |v: f64, o: f64, max: u32| ((v - o) * r.scale).round().clamp(0.0, f64::from(max)) as u32;
    let out: Vec<Cell> = areas
        .iter()
        .filter(|a| s.slices == SliceScope::All || a.id.is_some_and(|id| selected.contains(&id)))
        .map(|a| Cell {
            area: a,
            px: [
                edge(a.rect.x0, r.region.x0, r.width),
                edge(a.rect.y0, r.region.y0, r.height),
                edge(a.rect.x1, r.region.x0, r.width),
                edge(a.rect.y1, r.region.y0, r.height),
            ],
        })
        .filter(|c| c.px[2] > c.px[0] && c.px[3] > c.px[1])
        .collect();
    if out.is_empty() && s.slices == SliceScope::Selected {
        return Err("no selected slice in the image: select slices, or save all of them".into());
    }
    Ok(out)
}

/// The pixels of `px` (`[x0, y0, x1, y1]`) of `r`.
fn crop(r: &WebRender, [x0, y0, x1, y1]: [u32; 4]) -> Vec<u8> {
    let stride = r.width as usize * 4;
    (y0..y1)
        .flat_map(|y| r.rgba.get(y as usize * stride + x0 as usize * 4..y as usize * stride + x1 as usize * 4).unwrap_or_default())
        .copied()
        .collect()
}

/// What an export writes, by name relative to its folder.
pub struct WebFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The images folder of HTML and sliced output.
const IMAGES: &str = "images";

/// The files of an export of `doc` named after `stem`: one image (`<stem>.<ext>`), or one per
/// slice (`images/<slice name>.<ext>`), and with HTML output the page (`<stem>.html`, its image in
/// `images/`). The page comes first. Returns the image size too.
pub fn export_files(doc: &Document, selected: &[NodeId], s: &WebSettings, stem: &str) -> std::result::Result<(Vec<WebFile>, (u32, u32)), String> {
    let r = render(doc, s)?;
    let areas = doc.slice_layout();
    let cells = cells(&areas, selected, s, &r)?;
    let ext = s.format.ext();
    let html = s.output == WebOutput::Html;
    let mut images = vec![];
    // (cell, image file name) for the page.
    let mut placed: Vec<(Option<&Cell>, Option<String>)> = vec![];
    if cells.is_empty() {
        let name = if html { format!("{IMAGES}/{stem}.{ext}") } else { format!("{stem}.{ext}") };
        images.push(WebFile { name: name.clone(), bytes: optimize(doc, &r.rgba, r.width, r.height, s)?.bytes });
        placed.push((None, Some(name)));
    } else {
        let mut taken = HashSet::new();
        for c in &cells {
            let kind = c.area.id.and_then(|id| doc.slice_options(id)).map(|o| o.kind).unwrap_or_default();
            if kind != SliceKind::Image {
                placed.push((Some(c), None));
                continue;
            }
            let base: String = doc.slice_name(c.area).chars().map(|ch| if ch.is_alphanumeric() || "-_.".contains(ch) { ch } else { '-' }).collect();
            let base = unique_name(&base, |n| taken.contains(&n.to_lowercase()));
            taken.insert(base.to_lowercase());
            let name = format!("{IMAGES}/{base}.{ext}");
            let [x0, y0, x1, y1] = c.px;
            images.push(WebFile { name: name.clone(), bytes: optimize(doc, &crop(&r, c.px), x1 - x0, y1 - y0, s)?.bytes });
            placed.push((Some(c), Some(name)));
        }
    }
    let mut files = vec![];
    if html {
        files.push(WebFile { name: format!("{stem}.html"), bytes: page(doc, stem, (r.width, r.height), &placed, s).into_bytes() });
    }
    files.extend(images);
    Ok((files, (r.width, r.height)))
}

/// The HTML page placing the image or the slices (absolutely positioned, so overlapping slices
/// stack as laid out).
fn page(doc: &Document, stem: &str, (w, h): (u32, u32), placed: &[(Option<&Cell>, Option<String>)], s: &WebSettings) -> String {
    use super::fileio::imagemap::attr;
    use std::fmt::Write as _;
    let id = attr(stem);
    let mut out = String::new();
    let _ = writeln!(out, "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{id}</title>");
    let _ = writeln!(out, "<style>\nbody {{ margin: 0; }}\n.slices {{ position: relative; width: {w}px; height: {h}px; }}");
    let _ = writeln!(
        out,
        ".slices > * {{ position: absolute; display: block; overflow: hidden; }}\n.slices img {{ display: block; border: 0; }}\n</style>"
    );
    let _ = writeln!(out, "</head>\n<body>\n<div class=\"slices\" id=\"{id}\">");
    let matte = s.matte_rgb().ok().flatten().map(hex).unwrap_or_default();
    for (cell, image) in placed {
        let [x0, y0, x1, y1] = cell.map_or([0, 0, w, h], |c| c.px);
        let (cw, ch) = (x1 - x0, y1 - y0);
        let pos = format!("left: {x0}px; top: {y0}px; width: {cw}px; height: {ch}px;");
        let o = cell.and_then(|c| c.area.id).and_then(|i| doc.slice_options(i)).cloned().unwrap_or_default();
        match image {
            Some(src) => {
                let img = format!("<img src=\"{}\" width=\"{cw}\" height=\"{ch}\" alt=\"{}\">", attr(src), attr(&o.alt));
                if o.url.is_empty() {
                    let _ = writeln!(out, "<div style=\"{pos}\">{img}</div>");
                } else {
                    let target = if o.target.is_empty() { String::new() } else { format!(" target=\"{}\"", attr(&o.target)) };
                    let title = if o.message.is_empty() { String::new() } else { format!(" title=\"{}\"", attr(&o.message)) };
                    let _ = writeln!(out, "<a href=\"{}\"{target}{title} style=\"{pos}\">{img}</a>", attr(&o.url));
                }
            }
            None => {
                let bg = match o.background.as_str() {
                    "" => String::new(),
                    "matte" if !matte.is_empty() => format!(" background: {matte};"),
                    c if c.starts_with('#') => format!(" background: {};", attr(c)),
                    _ => String::new(),
                };
                let justify = match o.h_align {
                    CellAlign::Center => "center",
                    CellAlign::Right => "flex-end",
                    _ => "flex-start",
                };
                let align = match o.v_align {
                    CellVAlign::Middle => "center",
                    CellVAlign::Bottom => "flex-end",
                    CellVAlign::Baseline => "baseline",
                    _ => "flex-start",
                };
                // No Image: the cell's text as it is (HTML allowed); HTML Text: the type's text.
                let text = match o.kind {
                    SliceKind::HtmlText => cell
                        .and_then(|c| c.area.id)
                        .and_then(|i| doc.node(i))
                        .and_then(|n| match &n.kind {
                            NodeKind::Text(t) => Some(t.plain_text().split(['\n', '\r', '\u{2029}']).map(attr).collect::<Vec<_>>().join("<br>")),
                            _ => None,
                        })
                        .unwrap_or_default(),
                    _ => o.text.clone(),
                };
                let _ = writeln!(out, "<div style=\"{pos}{bg} display: flex; justify-content: {justify}; align-items: {align};\">{text}</div>");
            }
        }
    }
    out.push_str("</div>\n</body>\n</html>\n");
    out
}

// ---------- commands ----------

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let settings = s.web_settings(p).map_err(|e| bad(C, e))?;
    let st = s.doc()?;
    let selected = super::slices::selected_slices(&st.doc, &st.selection);
    let path = str_param(p, "path").filter(|p| !p.trim().is_empty());
    let stem = match path {
        Some(p) => super::fileio::file_stem(p),
        None => super::fileio::file_stem(&st.doc.title),
    };
    let stem = if stem.trim().is_empty() { "Untitled".to_string() } else { stem };
    let (files, (width, height)) = export_files(&st.doc, &selected, &settings, &stem).map_err(|e| bad(C, e))?;
    let total: usize = files.iter().map(|f| f.bytes.len()).sum();
    let Some(path) = path else {
        let rows: Vec<Value> = files.iter().map(|f| json!({ "name": f.name, "dataBase64": vectorcraft_format::base64_encode(&f.bytes) })).collect();
        return Ok(json!({ "files": rows, "bytes": total, "width": width, "height": height }));
    };
    // Siblings keep the path's own separators.
    let dir = &path[..path.rfind(['/', '\\']).map_or(0, |i| i + 1)];
    if files.iter().any(|f| f.name.contains('/')) {
        super::fileio::create_dir(&format!("{dir}{IMAGES}"))?;
    }
    let mut written = vec![];
    for f in &files {
        // A lone image goes where it was asked to.
        let target = if files.len() == 1 && !f.name.contains('/') { path.to_string() } else { format!("{dir}{}", f.name) };
        super::fileio::write_file(&target, &f.bytes)?;
        written.push(target);
    }
    Ok(json!({ "path": written.first(), "files": written, "bytes": total, "width": width, "height": height }))
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    const P: &str = "document.exportForWeb.preview";
    let settings = s.web_settings(p).map_err(|e| bad(P, e))?;
    let kbps = f64_or(p, "kbps", 56.6);
    if !(kbps.is_finite() && kbps > 0.0) {
        return Err(bad(P, format!("kbps must be a positive number, not {kbps}")));
    }
    let st = s.doc()?;
    let doc = &st.doc;
    let r = render(doc, &settings).map_err(|e| bad(P, e))?;
    let areas = doc.slice_layout();
    let selected = super::slices::selected_slices(doc, &st.selection);
    let cells = cells(&areas, &selected, &settings, &r).map_err(|e| bad(P, e))?;
    let images = cells.iter().filter(|c| c.area.id.and_then(|id| doc.slice_options(id)).is_none_or(|o| o.kind == SliceKind::Image)).count().max(1);
    let o = match p.get("slice").and_then(Value::as_u64) {
        Some(n) => {
            let c = cells.iter().find(|c| c.area.number as u64 == n).ok_or_else(|| bad(P, format!("no slice {n} in the image (see slice.list)")))?;
            let [x0, y0, x1, y1] = c.px;
            optimize(doc, &crop(&r, c.px), x1 - x0, y1 - y0, &settings)
        }
        None => optimize(doc, &r.rgba, r.width, r.height, &settings),
    }
    .map_err(|e| bad(P, e))?;
    let mut out = json!({
        "format": settings.format.id(), "width": o.width, "height": o.height, "bytes": o.bytes.len(),
        "seconds": download_seconds(o.bytes.len(), kbps), "kbps": kbps, "slices": images,
    });
    if settings.format.palette() {
        out["colors"] = o
            .colors
            .iter()
            .map(|c| {
                json!({
                    "color": hex(c.color), "source": hex(c.source), "transparent": c.transparent, "locked": c.locked,
                    "webShifted": c.web_shifted, "webSafe": quantize::web_safe(c.color) == c.color,
                })
            })
            .collect();
        out["mappedToTransparent"] = o.mapped.iter().map(|c| json!(hex(*c))).collect();
    }
    if bool_or(p, "image", false) {
        out["dataBase64"] = json!(vectorcraft_format::base64_encode(&o.bytes));
    }
    Ok(out)
}

fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    const S: &str = "webExport.settings";
    let base = if bool_or(p, "reset", false) { WebSettings::default() } else { s.last_web_settings() };
    let next = s.web_settings_over(&base, p).map_err(|e| bad(S, e))?;
    if p.as_object().is_some_and(|o| !o.is_empty()) {
        s.prefs.web_export_settings = Some(next.clone());
    }
    Ok(json!({ "settings": next.to_json() }))
}

fn presets_list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows = builtin_presets()
        .into_iter()
        .map(|p| (p, true))
        .chain(s.prefs.web_export_presets.iter().cloned().map(|p| (p, false)))
        .map(|(p, builtin)| json!({ "name": p.name, "builtIn": builtin, "settings": p.settings.to_json() }));
    Ok(json!({ "presets": rows.collect::<Vec<_>>() }))
}

fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    const S: &str = "webExport.presets.save";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(S, "give the preset a `name`"))?.to_string();
    if is_builtin(&name) {
        return Err(bad(S, format!("`{name}` is a built-in preset and can't change: save it under another name")));
    }
    let at = saved_index(&s.prefs.web_export_presets, &name);
    let base = at.and_then(|i| s.prefs.web_export_presets.get(i)).map(|q| q.settings.clone()).unwrap_or_default();
    // Settings at the top level, or in `settings`.
    let mut keys = p.as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(inner)) = keys.remove("settings") {
        keys.extend(inner);
    }
    let settings = s.web_settings_over(&base, &Value::Object(keys)).map_err(|e| bad(S, e))?;
    let name = match str_param(p, "newName").map(str::trim) {
        Some("") => return Err(bad(S, "`newName` is empty")),
        Some(n) if is_builtin(n) || saved_index(&s.prefs.web_export_presets, n).is_some_and(|i| Some(i) != at) => {
            return Err(bad(S, format!("a preset named `{n}` exists")));
        }
        Some(n) => n.to_string(),
        None => at.and_then(|i| s.prefs.web_export_presets.get(i)).map_or(name, |q| q.name.clone()),
    };
    let preset = WebPreset { name: name.clone(), settings: settings.clone() };
    match at.and_then(|i| s.prefs.web_export_presets.get_mut(i)) {
        Some(slot) => *slot = preset,
        None => s.prefs.web_export_presets.push(preset),
    }
    Ok(json!({ "name": name, "settings": settings.to_json(), "created": at.is_none() }))
}

fn presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const S: &str = "webExport.presets.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(S, "missing `name`"))?;
    if is_builtin(name) {
        return Err(bad(S, format!("`{name}` is a built-in preset and stays")));
    }
    let i = saved_index(&s.prefs.web_export_presets, name).ok_or_else(|| bad(S, format!("no saved preset named `{name}`")))?;
    Ok(json!({ "deleted": s.prefs.web_export_presets.remove(i).name }))
}
