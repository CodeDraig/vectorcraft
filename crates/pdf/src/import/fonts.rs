//! PDF character maps, glyph comparison and installed-font matching.

use super::{Arc, BfString, HashMap, Rect, Shape};

/// The characters of a CID-keyed CFF font embedded without a ToUnicode map (as macOS writes
/// Hiragino): each glyph's CID from the font's charset, then Adobe's CID → Unicode table of its
/// character collection (Japan1, GB1, CNS1, Korea1).
pub(crate) struct CidText {
    /// CID of each glyph.
    cids: Vec<u16>,
    table: hayro_interpret::hayro_cmap::CMap,
}

impl CidText {
    pub(crate) fn of(data: &[u8]) -> Option<Self> {
        // An embedded CFF of a few CJK glyphs is kilobytes; whole fonts are some megabytes.
        if data.len() > 64 << 20 {
            return None;
        }
        use hayro_interpret::hayro_cmap::{CMap, CMapName, load_embedded};
        use skrifa::raw::ps::cff::{CffFontRef, dict, v1::Cff};
        use skrifa::raw::{FontData, FontRead};
        let cff = Cff::read(FontData::new(data)).ok()?;
        let top = cff.top_dicts().get(0)?;
        let ordering = dict::entries(top, None).filter_map(Result::ok).find_map(|e| match e {
            dict::Entry::Ros { ordering, .. } => Some(ordering),
            _ => None,
        })?;
        let name = match cff.string(ordering)? {
            b"Japan1" => CMapName::AdobeJapan1Ucs2,
            b"GB1" => CMapName::AdobeGb1Ucs2,
            b"CNS1" => CMapName::AdobeCns1Ucs2,
            b"Korea1" => CMapName::AdobeKorea1Ucs2,
            _ => return None,
        };
        let table = CMap::parse(load_embedded(name)?, load_embedded)?;
        let charset = CffFontRef::new(data, 0, None).ok()?.charset()?;
        let cids = (0..charset.num_glyphs().min(65_536)).map(|g| charset.string_id(skrifa::GlyphId::new(g)).map_or(0, |s| s.to_u16())).collect();
        Some(Self { cids, table })
    }

    pub(super) fn unicode(&self, glyph: u32) -> Option<hayro_interpret::hayro_cmap::BfString> {
        let cid = *self.cids.get(glyph as usize)?;
        self.table.lookup_bf_string(u32::from(cid))
    }
}

/// A ToUnicode value that names no character: U+FFFD alone (#708).
pub(super) fn replacement(u: &hayro_interpret::hayro_cmap::BfString) -> bool {
    use hayro_interpret::hayro_cmap::BfString;
    match u {
        BfString::Char(c) => *c == char::REPLACEMENT_CHARACTER,
        BfString::String(s) => !s.is_empty() && s.chars().all(|c| c == char::REPLACEMENT_CHARACTER),
    }
}

/// The character of each glyph of an embedded TrueType or OpenType font program, from its own
/// `cmap` (the first character mapped to each glyph); `None` for a bare CFF or a font without one.
pub(crate) fn font_chars(data: &[u8]) -> Option<HashMap<u32, char>> {
    use skrifa::MetadataProvider;
    // As for CidText: whole fonts are some megabytes.
    if data.len() > 64 << 20 {
        return None;
    }
    let font = skrifa::FontRef::new(data).ok()?;
    let mut chars = HashMap::new();
    for (c, g) in font.charmap().mappings() {
        if let Some(c) = char::from_u32(c).filter(|c| !c.is_control()) {
            chars.entry(g.to_u32()).or_insert(c);
        }
    }
    (!chars.is_empty()).then_some(chars)
}

/// The ideographs of the Kangxi radicals U+2F00–U+2FD5 (their NFKC forms): the CID → Unicode
/// tables of some PDFs map 龍 to the radical ⿓, which looks the same but isn't the character.
const KANGXI: &str = "一丨丶丿乙亅二亠人儿入八冂冖冫几凵刀力勹匕匚匸十卜卩厂厶又口囗土士夂夊夕大女子宀寸小尢尸屮山巛工己巾干幺广廴廾弋弓彐彡彳心戈戶手支攴文斗斤方无日曰月木欠止歹殳毋比毛氏气水火爪父爻爿片牙牛犬玄玉瓜瓦甘生用田疋疒癶白皮皿目矛矢石示禸禾穴立竹米糸缶网羊羽老而耒耳聿肉臣自至臼舌舛舟艮色艸虍虫血行衣襾見角言谷豆豕豸貝赤走足身車辛辰辵邑酉釆里金長門阜隶隹雨靑非面革韋韭音頁風飛食首香馬骨高髟鬥鬯鬲鬼魚鳥鹵鹿麥麻黃黍黑黹黽鼎鼓鼠鼻齊齒龍龜龠";

/// How far glyph `o` (the PDF's, for `text`) is from the installed `face`'s glyph for it: the
/// largest difference between their boxes, in thousandths of an em. `None` when they are clearly
/// different glyphs (or the face lacks the character); `Some(0)` when there is nothing to compare
/// (several characters, a vertical form, no ink in either).
fn glyph_deviation(face: &vectorcraft_text::FontFace, text: &str, o: &hayro_interpret::font::OutlineGlyph, vertical: bool) -> Option<f64> {
    let Some(c) = comparable(text, vertical) else { return Some(0.0) };
    let gid = face.glyph_for(c);
    if gid == 0 {
        return None;
    }
    let embedded = o.outline();
    let installed = vectorcraft_text::FontDb::global().outline(face, gid);
    match (embedded.elements().is_empty(), installed.elements().is_empty()) {
        (true, true) => Some(0.0),
        (false, false) => {
            let e = embedded.bounding_box();
            let k = 1000.0 / face.units_per_em();
            // Installed outlines are y-down; the PDF's glyph space is y-up.
            let i = installed.bounding_box();
            let i = Rect::new(i.x0 * k, -i.y1 * k, i.x1 * k, -i.y0 * k);
            let off = [(e.x0, i.x0), (e.x1, i.x1), (e.y0, i.y0), (e.y1, i.y1)].iter().fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
            (off <= 100.0).then_some(off)
        }
        _ => None,
    }
}

/// The face of `face`'s family that draws glyph `o` (for `text`): `face` itself, or the style whose
/// glyph is closest (a variable font's named instances, or a family whose PostScript names don't
/// tell its styles apart, draw a character differently in each style). `None`: no style of the
/// family has that glyph, so the font isn't the installed one.
pub(super) fn matching_face(
    face: &Arc<vectorcraft_text::FontFace>,
    text: &str,
    o: &hayro_interpret::font::OutlineGlyph,
    vertical: bool,
) -> Option<Arc<vectorcraft_text::FontFace>> {
    let own = glyph_deviation(face, text, o, vertical);
    if own == Some(0.0) {
        return Some(face.clone());
    }
    let db = vectorcraft_text::FontDb::global();
    // Other installed versions of its family and style (a font library keeps old ones), then the
    // family's other styles.
    let versions = db.versions(face).into_iter().skip(1).take(16);
    let styles =
        db.styles(&face.family).into_iter().filter(|s| !s.eq_ignore_ascii_case(&face.style)).take(64).filter_map(|s| db.face(&face.family, &s));
    let others = versions.chain(styles);
    let mut best = own.map(|d| (d, face.clone()));
    for f in others {
        if let Some(d) = glyph_deviation(&f, text, o, vertical)
            && best.as_ref().is_none_or(|(b, _)| d < *b)
        {
            best = Some((d, f));
        }
    }
    best.map(|(_, f)| f)
}

/// The character of glyph `text` when its outline can be compared with an installed font's: a
/// single character, and not CJK punctuation set vertically (its vertical form differs).
pub(super) fn comparable(text: &str, vertical: bool) -> Option<char> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    if vertical && matches!(c as u32, 0x2014 | 0x2015 | 0x2025 | 0x2026 | 0x3000..=0x303F | 0x30FC | 0xFE30..=0xFE4F | 0xFF00..=0xFF65) {
        return None;
    }
    Some(c)
}

pub(crate) fn unify_radical(c: char) -> char {
    let i = (c as u32).wrapping_sub(0x2F00);
    if i < 214 { KANGXI.chars().nth(i as usize).unwrap_or(c) } else { c }
}

/// The font a PDF font draws with ([`Builder::font_name`]).
#[derive(Clone)]
pub(super) struct FontInfo {
    family: String,
    style: String,
    /// The installed face of its PostScript name.
    face: Option<Arc<vectorcraft_text::FontFace>>,
}

use super::{Affine, Builder, Families, Look, Placement, StrokeProps, TextAs, TextLine, Upright};

impl<'p> Builder<'p> {
    /// The family and style of font `key` (drawing glyph `o`): the installed face of its PostScript
    /// name, else the available family its name reads as; whether it is available.
    pub(super) fn font_name(&mut self, key: u128, o: &hayro_interpret::font::OutlineGlyph) -> FontInfo {
        if let Some(n) = self.font_names.get(&key) {
            return n.clone();
        }
        let data = o.font_data();
        let name = self.fonts.get(&key).cloned().or_else(|| data.as_ref().and_then(|d| d.postscript_name.clone()));
        let (weight, italic) = data.as_ref().map_or((None, false), |d| (d.weight, d.is_italic));
        // A subset's six-letter tag.
        let ps = name.as_deref().map(|n| match n.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest.to_string(),
            _ => n.to_string(),
        });
        let installed = ps.as_deref().and_then(|ps| vectorcraft_text::FontDb::global().find_postscript(ps));
        let n = match (installed, name) {
            (Some(face), _) => FontInfo { family: face.family.clone(), style: face.style.clone(), face: Some(face) },
            (None, Some(name)) => {
                let families = self.families.get_or_insert_with(Families::available);
                let (family, style, found) = families.resolve(&name, weight, italic);
                if !found && !family.is_empty() && !self.missing_fonts.contains(&family) {
                    self.missing_fonts.push(family.clone());
                }
                FontInfo { family, style, face: None }
            }
            (None, None) => {
                let d = vectorcraft_doc::CharStyle::default();
                FontInfo { family: d.font_family, style: d.font_style, face: None }
            }
        };
        self.font_names.insert(key, n.clone());
        n
    }

    /// Glyph `o` as type: added to the line being gathered (or starting one). `false`: it keeps
    /// its outline (no Unicode text, mirrored, or not type mode).
    pub(super) fn type_glyph(
        &mut self,
        o: &hayro_interpret::font::OutlineGlyph,
        m: Affine,
        scale: f64,
        paint: &hayro_interpret::Paint<'_>,
        stroke: Option<&StrokeProps>,
    ) -> bool {
        if self.text_as != TextAs::Text || self.nested > 0 {
            return false;
        }
        let key = o.font_cache_key();
        let glyph = o.glyph_id().to_u32();
        let (unicode, from_cid) = match o.as_unicode().filter(|u| !replacement(u)) {
            Some(u) => (Some(u), false),
            // No mapping, or one to U+FFFD (#708): the font program may still name the character.
            None => {
                let cid = self.cid_text.entry(key).or_insert_with(|| o.font_data().and_then(|d| CidText::of(d.data.as_ref().as_ref())).map(Arc::new));
                match cid.as_ref().and_then(|c| c.unicode(glyph)) {
                    Some(u) => (Some(u), true),
                    None => {
                        let chars = self
                            .font_chars
                            .entry(key)
                            .or_insert_with(|| o.font_data().and_then(|d| font_chars(d.data.as_ref().as_ref())).map(Arc::new));
                        (chars.as_ref().and_then(|m| m.get(&glyph)).map(|c| BfString::Char(*c)), false)
                    }
                }
            }
        };
        let text: String = match unicode {
            Some(BfString::Char(c)) => c.to_string(),
            Some(BfString::String(s)) => s,
            None => {
                if o.as_unicode().is_some_and(|u| replacement(&u)) {
                    let name = self.font_name(key, o).family;
                    self.warn(&format!("text in {name} whose characters the file doesn't name (U+FFFD) was kept as outlines"));
                }
                return false;
            }
        };
        // Adobe's CID tables map some ideographs to their look-alike Kangxi radicals.
        let text: String = if from_cid { text.chars().map(unify_radical).collect() } else { text };
        let Some(at) = Placement::of(m) else { return false };
        if text.is_empty() || text.chars().any(|c| c.is_control()) {
            return false;
        }
        let width = o.advance_width().filter(|w| w.is_finite());
        // Set vertically (WMode 1): no horizontal advance; its em box is a full em wide.
        let upright = width.is_some_and(|w| w.abs() <= 1.0);
        let top = m * kurbo::Point::new(f64::from(width.filter(|w| *w > 1.0).unwrap_or(1000.0)) * 0.5, 880.0);
        let mut info = self.font_name(key, o);
        if let Some(face) = &info.face {
            let decided = match self.matched.get(&key) {
                Some(m) => m.clone(),
                // A glyph that can't be compared (several characters, a vertical form) decides
                // nothing: it takes the face of its name until one that can be compared does.
                None if comparable(&text, upright).is_none() => Some(face.clone()),
                None => {
                    let m = matching_face(face, &text, o, upright);
                    self.matched.insert(key, m.clone());
                    m
                }
            };
            let Some(f) = decided else {
                self.warn("text whose glyphs differ from the installed font of the same name was kept as outlines");
                return false;
            };
            info = FontInfo { family: f.family.clone(), style: f.style.clone(), face: Some(f) };
        }
        // A version of the family and style that the two alone don't resolve to: the type names it.
        // Kept per font once its face is decided (until then each glyph asks).
        let named = |f: &Option<Arc<vectorcraft_text::FontFace>>| {
            let f = f.as_ref()?;
            let default = vectorcraft_text::FontDb::global().face(&f.family, &f.style)?;
            (default.version != f.version).then(|| f.version.clone())
        };
        let version = match self.versions.get(&key) {
            Some(v) => v.clone(),
            None if self.matched.contains_key(&key) => self.versions.entry(key).or_insert_with(|| named(&info.face)).clone(),
            None => named(&info.face),
        };
        let (paint, opacity) = self.paint(paint, stroke.is_some());
        let stroke = stroke.map(|p| (paint.clone(), p.line_width as f64 * scale));
        let look = Look {
            font: key,
            family: info.family,
            style: info.style,
            version,
            size: at.size,
            h_scale: at.h_scale,
            fill: stroke.is_none().then_some(paint),
            stroke,
        };
        let advance = width.map_or(at.size * 0.5, |w| w as f64 / 1000.0 * at.size * at.h_scale / 100.0);
        let place = |line: &mut TextLine| {
            if upright { line.push_upright(&look, at, opacity, Upright { top }, &text) } else { line.push(&look, at, opacity, advance, &text) }
        };
        self.flush_glyphs();
        // A stroke over the glyph just filled (fill and stroke rendering).
        if let (Some(s), Some(line)) = (&look.stroke, &mut self.text)
            && look.fill.is_none()
            && line.stroke_last(key, at, s.clone())
        {
            return true;
        }
        if !self.text.as_mut().is_some_and(place) {
            self.flush_text();
            let mut line = TextLine::new(at, opacity);
            place(&mut line);
            self.text = Some(line);
        }
        // Its ink: a stroke the file draws as outlines of the glyphs is told by it.
        let mut ink = o.outline();
        if let Some(line) = &mut self.text
            && !ink.elements().is_empty()
        {
            ink.apply_affine(m);
            line.ink.push(ink.bounding_box());
        }
        true
    }
}
