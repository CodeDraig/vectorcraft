//! The SVG writer.

mod paint;
mod resources;

mod text;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use vectorcraft_color::{BlendMode, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    AppearanceItem, Document, FillLayer, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeAlign, StrokeLayer, TextKind, TextObject,
};
use vectorcraft_doc::{CharStyle, ImageBlob, Justify};
use vectorcraft_effects::stroke::{self, Written, WrittenShape};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect};

use vectorcraft_text::{FontDb, FontFace};

use crate::{
    EDITING_NS, Encoding, ExportOptions, ImageMode, LinkedImage, ObjectIds, Output, Profile, Styling, base64_encode, body_hash, fmt_num, fnv1a,
    xml_escape,
};

use crate::css::{self, Props, blend_css, css_string, font_descriptor};

pub(crate) fn export(doc: &Document, opts: &ExportOptions, native: Option<&[u8]>) -> Output {
    // Live geometry effects (Roughen, Warp, Offset Path, Effect → Pathfinder…) export as their result.
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    if opts.object_ids != ObjectIds::Unique {
        return write(doc, opts, native, "");
    }
    // Unique ids: every id and class name gets a prefix hashed from the output written without
    // one, so the same document always gets the same ids and different ones never share any.
    let plain = write(doc, opts, native, "");
    let prefix = format!("u{:010x}-", fnv1a(plain.svg.as_bytes()) & 0xff_ffff_ffff);
    write(doc, opts, native, &prefix)
}

fn write(doc: &Document, opts: &ExportOptions, native: Option<&[u8]>, id_prefix: &str) -> Output {
    let rect = opts
        .artboard
        .and_then(|i| doc.artboards.get(i))
        .map(|a| a.rect)
        .or_else(|| doc.art_bounds())
        .or_else(|| doc.artboards.first().map(|a| a.rect))
        .unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
    let mut w = Writer {
        doc,
        opts,
        xf: Affine::translate((-rect.x0, -rect.y0)),
        body: String::new(),
        defs: String::new(),
        classes: Vec::new(),
        used_ids: HashSet::new(),
        names: HashMap::new(),
        depth: 1,
        patterns: HashMap::new(),
        gradients: HashMap::new(),
        pattern_nest: 0,
        brushes: None,
        knockout: doc.page_knockout,
        anonymous: false,
        knockout_filter: None,
        warnings: vec![],
        id_prefix,
        linked: Vec::new(),
        instance_xf: Affine::IDENTITY,
        symbols: HashMap::new(),
        symbol_nest: 0,
        fonts: Vec::new(),
        shared_images: HashMap::new(),
        layer_nest: 0,
    };
    w.assign_name_ids();
    // Page Isolated Blending / Page Knockout Group: the page content is one isolated group.
    let page_group = doc.page_isolate || doc.page_knockout;
    if page_group {
        let a = w.attrs(&vec![("isolation", "isolate".into())]);
        w.line(&format!("<g{a}>"));
        w.depth += 1;
    }
    w.children(&doc.layers);
    if page_group {
        w.depth -= 1;
        w.line("</g>");
    }

    let nl = if opts.minify { "" } else { "\n" };
    let mut out = String::new();
    // A file in neither UTF-8 nor UTF-16 must name its encoding.
    if !opts.minify || opts.encoding == Encoding::Latin1 {
        out.push_str(&format!("<?xml version=\"1.0\" encoding=\"{}\"?>{nl}", opts.encoding.xml_name()));
    }
    if opts.styling == Styling::StyleEntities && !w.classes.is_empty() {
        out.push_str("<!DOCTYPE svg [");
        for (i, c) in w.classes.iter().enumerate() {
            out.push_str(&format!("{nl}{}<!ENTITY st{} \"{}\">", w.indent(1), i + 1, entity_value(c)));
        }
        out.push_str(&format!("{nl}]>{nl}"));
    }
    let (ww, hh) = (w.num(rect.width()), w.num(rect.height()));
    out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
    if w.tiny() {
        out.push_str(" version=\"1.2\" baseProfile=\"tiny\"");
    }
    if !opts.responsive {
        out.push_str(&format!(" width=\"{ww}\" height=\"{hh}\""));
    }
    out.push_str(&format!(" viewBox=\"0 0 {ww} {hh}\">{nl}"));
    let css = opts.styling == Styling::InternalCss && !w.classes.is_empty();
    let fonts = w.font_faces();
    if !w.defs.is_empty() || css || !fonts.is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<defs>{nl}"));
        if css || !fonts.is_empty() {
            out.push_str(&w.indent(2));
            out.push_str("<style>");
            let classes = w.classes.iter().enumerate().filter(|_| css).map(|(i, c)| format!(".{id_prefix}cls-{}{{{c}}}", i + 1));
            for rule in classes.chain(fonts) {
                out.push_str(&format!("{nl}{}{}", w.indent(3), xml_escape(&rule)));
            }
            out.push_str(&format!("{nl}{}</style>{nl}", w.indent(2)));
        }
        out.push_str(&w.defs);
        out.push_str(&w.indent(1));
        out.push_str(&format!("</defs>{nl}"));
    }
    if !doc.title.is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<title>{}</title>{nl}", xml_escape(&doc.title)));
    }
    // File Info's description.
    if !doc.metadata.description.trim().is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<desc>{}</desc>{nl}", xml_escape(&doc.metadata.description)));
    }
    let latin1 = opts.encoding == Encoding::Latin1;
    if latin1 {
        latin1_refs(&mut out);
        latin1_refs(&mut w.body);
    }
    // The editing data carries a hash of the markup around its `<metadata>`.
    let native = native.filter(|_| opts.preserve_editing).map(|bytes| (bytes, body_hash(&[&out, &w.body, "</svg>", nl])));
    let mut meta = String::new();
    w.metadata(&mut meta, native);
    if latin1 {
        latin1_refs(&mut meta);
    }
    out.push_str(&meta);
    out.push_str(&w.body);
    out.push_str("</svg>");
    out.push_str(nl);
    Output { svg: out, linked: w.linked, warnings: w.warnings, encoding: opts.encoding }
}

/// Write every character of `s` beyond ISO 8859-1 as a character reference (all of them sit in
/// attribute values, text or style sheets, where references read as the character).
fn latin1_refs(s: &mut String) {
    if s.chars().all(|c| u32::from(c) < 0x100) {
        return;
    }
    let mut o = String::with_capacity(s.len() + 64);
    for c in s.chars() {
        if u32::from(c) < 0x100 {
            o.push(c);
        } else {
            let _ = write!(o, "&#x{:X};", u32::from(c));
        }
    }
    *s = o;
}

/// `chars` (sorted) as a CSS `unicode-range` (`U+41-5A,U+61`).
fn unicode_range(chars: &BTreeSet<char>) -> String {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for c in chars.iter().map(|c| u32::from(*c)) {
        match runs.last_mut() {
            Some((_, end)) if c == *end + 1 => *end = c,
            _ => runs.push((c, c)),
        }
    }
    let one = |(a, b): (u32, u32)| if a == b { format!("U+{a:X}") } else { format!("U+{a:X}-{b:X}") };
    runs.into_iter().map(one).collect::<Vec<_>>().join(",")
}

/// A CSS declaration block as the literal value of an `<!ENTITY>`: quotes, `%` and markup
/// characters become references that still read as themselves inside the `style` attribute.
fn entity_value(decl: &str) -> String {
    let mut o = String::with_capacity(decl.len());
    for c in decl.chars() {
        match c {
            '"' => o.push_str("&#34;"),
            '%' => o.push_str("&#37;"),
            // Doubly escaped: the entity's replacement text is parsed again where it is used.
            '&' => o.push_str("&#38;#38;"),
            '<' => o.push_str("&#38;#60;"),
            _ => o.push(c),
        }
    }
    o
}

/// Image `b` as SVG viewers show it: PNG, JPEG, GIF, WebP and SVG as they are, other formats (a
/// CMYK TIFF) as PNG.
fn web_image(b: &ImageBlob) -> std::borrow::Cow<'_, ImageBlob> {
    use std::borrow::Cow;
    if matches!(b.mime.as_str(), "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" | "image/svg+xml") {
        return Cow::Borrowed(b);
    }
    let png = image::load_from_memory(&b.bytes).ok().and_then(|img| {
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
        Some(out)
    });
    png.map_or(Cow::Borrowed(b), |png| Cow::Owned(ImageBlob::png(png)))
}

/// The usual file extension of an image MIME type.
fn image_ext(mime: &str) -> &str {
    match mime {
        "image/jpeg" => "jpg",
        "image/svg+xml" => "svg",
        m => m.strip_prefix("image/").filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin"),
    }
}

struct Writer<'a> {
    doc: &'a Document,
    opts: &'a ExportOptions,
    /// Document → SVG user space (artboard origin translation, symbol instance transforms).
    xf: Affine,
    body: String,
    defs: String,
    /// Unique CSS declaration blocks; class name is `cls-{index + 1}`.
    classes: Vec<String>,
    used_ids: HashSet<String>,
    names: HashMap<NodeId, String>,
    depth: usize,
    /// `<pattern>` def ids by pattern name + placement.
    patterns: HashMap<String, String>,
    /// Gradient def ids by content (everything but the id).
    gradients: HashMap<String, String>,
    pattern_nest: u32,
    /// Whether the group being written is a knockout group (what its neutral children inherit).
    knockout: bool,
    /// Writing a copy of an object (a knockout mask): no object ids, so they stay unique.
    anonymous: bool,
    /// The filter that paints art black keeping its alpha (knockout masks), once defined.
    knockout_filter: Option<String>,
    /// The brush library, parsed when the first brushed stroke is written.
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// Features written approximately (once each).
    warnings: Vec<String>,
    /// Prepended to every id and class name ([`ObjectIds::Unique`]).
    id_prefix: &'a str,
    /// Image files the SVG links to ([`ImageMode::Link`]).
    linked: Vec<LinkedImage>,
    /// The transform of the symbol instances written out as their own art around the object
    /// being written (document space; identity outside them and in symbol defs).
    instance_xf: Affine,
    /// What each symbol's instances can share, by symbol name (see [`Writer::symbol_use`]).
    symbols: HashMap<String, SymbolDef>,
    /// Symbols being written inside one another (instances nested deeper are left out: a symbol
    /// can't contain itself).
    symbol_nest: u32,
    /// The characters type uses from each face ([`ExportOptions::embed_fonts`]).
    fonts: Vec<FontUse>,
    /// Images written once in the defs and drawn with `<use>`, by key and size: the pieces an
    /// envelope cuts a distorted image into share its pixels.
    shared_images: HashMap<(String, u32, u32), String>,
    /// Layers being written inside one another (more than one: a sublayer).
    layer_nest: usize,
}

/// The characters type uses from one face, under one `@font-face` description: the family the
/// type names and the weight and italic its style asks for.
struct FontUse {
    family: String,
    weight: u16,
    italic: bool,
    face: Arc<FontFace>,
    chars: BTreeSet<char>,
}

/// A symbol's `<symbol>` def and the instance transforms that can `<use>` it.
struct SymbolDef {
    /// The def's id, once written.
    id: Option<String>,
    reuse: Reuse,
}

/// Which instance transforms paint a symbol's art exactly as the def under that transform (the
/// canvas transforms the art's geometry alone: stroke weights, effects, patterns and unlinked
/// masks stay put).
#[derive(Clone, Copy)]
pub(crate) struct Reuse {
    /// Translations (no pattern paints or unlinked masks).
    moves: bool,
    /// Rotations and reflections (nor effects or brushes).
    turns: bool,
    /// Any other transform (nor strokes or live objects).
    scales: bool,
}

/// How one stroke is written.
enum StrokePlan {
    /// Its brush art (document space).
    Brush(Vec<Node>),
    Written(Written),
}

/// How deep symbols may sit in one another's art (deeper instances are left out).
const MAX_SYMBOL_NEST: u32 = 8;

impl Reuse {
    const NONE: Self = Self { moves: false, turns: false, scales: false };

    /// What a def of symbol art `art` stands for.
    pub(crate) fn of(doc: &Document, art: &Node) -> Self {
        let mut r = Self { moves: true, turns: true, scales: true };
        r.scan(doc, art, 0);
        r
    }

    /// Narrow down to what `n` (inside symbols nested `depth` deep) allows.
    fn scan(&mut self, doc: &Document, n: &Node, depth: u32) {
        let pattern = |p: &Paint| matches!(p, Paint::Pattern { .. });
        let text_patterns = matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| pattern(&r.style.fill) || pattern(&r.style.stroke)));
        let unlinked_mask = n.mask.as_ref().is_some_and(|m| !m.linked);
        if depth > MAX_SYMBOL_NEST || text_patterns || unlinked_mask || vectorcraft_doc::pattern::uses_pattern(n, None) {
            *self = Self::NONE;
            return;
        }
        let effects = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible);
        if effects(&n.appearance.effects) || n.appearance.items.iter().any(|i| effects(i.effects())) {
            (self.turns, self.scales) = (false, false);
        }
        for i in &n.appearance.items {
            if let AppearanceItem::Stroke(s) = i
                && s.visible
                && !s.paint.is_none()
                && s.width > 0.0
            {
                // Stroke weights don't scale; brushes can orient their art to the page.
                self.scales = false;
                self.turns &= s.brush.is_none();
            }
        }
        match &n.kind {
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::PlacedDocument(_) => {
                (self.turns, self.scales) = (false, false)
            }
            NodeKind::SymbolInstance { symbol, .. } => {
                if let Some(s) = doc.symbols.iter().find(|s| s.name == *symbol) {
                    self.scan(doc, &s.art, depth + 1);
                }
            }
            _ => {}
        }
        if let Some(m) = &n.mask {
            self.scan(doc, &m.art, depth);
        }
        for c in n.children().into_iter().flatten() {
            self.scan(doc, c, depth);
        }
    }

    /// Can the def stand for an instance with transform `xf`?
    pub(crate) fn allows(self, xf: Affine) -> bool {
        const EPS: f64 = 1e-9;
        let [a, b, c, d, _, _] = xf.as_coeffs();
        if (a - 1.0).abs() < EPS && b.abs() < EPS && c.abs() < EPS && (d - 1.0).abs() < EPS {
            return self.moves;
        }
        let rigid = (a * a + b * b - 1.0).abs() < EPS && (c * c + d * d - 1.0).abs() < EPS && (a * c + b * d).abs() < EPS;
        if rigid { self.turns } else { self.scales }
    }
}

/// Art about to be written moved by `m.inverse()` (a symbol instance's): map what the canvas
/// leaves on the page by `m` first, so it stays there. That is every pattern paint's placement
/// and the art of unlinked opacity masks.
fn pin_to_page(n: &mut Node, m: Affine) {
    vectorcraft_doc::pattern::transform_pattern_paints(n, m);
    if let Some(mask) = n.mask.as_deref_mut() {
        let linked = mask.linked;
        let art = std::sync::Arc::make_mut(&mut mask.art);
        if !linked {
            art.transform(m, false);
        }
        pin_to_page(art, m);
    }
    for c in n.children_mut().into_iter().flatten() {
        pin_to_page(std::sync::Arc::make_mut(c), m);
    }
}

/// The Dublin Core terms File Info writes to `<metadata>`: the format, title, author (creator),
/// description, each keyword (subject), copyright notice and URL (rights) and created date, the
/// empty ones left out.
fn dublin_core(doc: &Document) -> Vec<(&'static str, String)> {
    let info = &doc.metadata;
    let mut terms = vec![("format", "image/svg+xml".to_string())];
    let created = info.created.map(vectorcraft_doc::metadata::iso8601).unwrap_or_default();
    let fields = [("title", doc.title.as_str()), ("creator", info.author.as_str()), ("description", info.description.as_str())];
    let rights = [("rights", info.copyright_notice.as_str()), ("rights", info.copyright_url.as_str()), ("date", created.as_str())];
    let keywords = info.keywords.iter().map(|k| ("subject", k.as_str()));
    terms.extend(fields.into_iter().chain(keywords).chain(rights).filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k, v.to_string())));
    terms
}

/// What SVG Tiny 1.2 does without masks.
const TINY_MASKS: &str = "SVG Tiny 1.2 has no masks: opacity masks are left out and knockout groups written as plain groups";
/// A region covering any artwork (mask and filter extents).
const BIG: &str = "x=\"-100000\" y=\"-100000\" width=\"200000\" height=\"200000\"";
/// Attributes of every `<mask>`: user-space units, and luminance taken from the sRGB values (as the
/// canvas and PDF take it) rather than from linearised ones.
const MASK: &str = "maskUnits=\"userSpaceOnUse\" color-interpolation=\"sRGB\"";
/// Attribute of an exported opacity mask listing its options that differ from the defaults
/// (clipping, not inverted): `noclip`, `invert`.
pub(crate) const MASK_FLAGS: &str = "data-vectorcraft-mask";

/// Turn an object name into a valid, readable XML id.
pub(crate) fn sanitize_id(name: &str) -> String {
    let mut s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    if s.is_empty() || !s.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        s.insert(0, '_');
    }
    s
}

impl Writer<'_> {
    /// Writing the SVG Tiny 1.2 profile?
    fn tiny(&self) -> bool {
        self.opts.profile == Profile::Tiny12
    }
    fn num(&self, v: f64) -> String {
        fmt_num(v, self.opts.decimals)
    }
    fn indent(&self, depth: usize) -> String {
        if self.opts.minify { String::new() } else { "  ".repeat(depth) }
    }
    fn line(&mut self, s: &str) {
        let ind = self.indent(self.depth);
        self.body.push_str(&ind);
        self.body.push_str(s);
        if !self.opts.minify {
            self.body.push('\n');
        }
    }
    fn def(&mut self, depth: usize, s: &str) {
        let ind = self.indent(depth + 1);
        self.defs.push_str(&ind);
        self.defs.push_str(s);
        if !self.opts.minify {
            self.defs.push('\n');
        }
    }

    fn fresh_id(&mut self, prefix: &str) -> String {
        let mut i = 1;
        loop {
            let id = format!("{}{prefix}-{i}", self.id_prefix);
            if self.used_ids.insert(id.clone()) {
                return id;
            }
            i += 1;
        }
    }
    /// `base` (with the unique prefix) if free, else `base-2`, `base-3`…
    fn unique_id(&mut self, base: &str) -> String {
        let base = format!("{}{base}", self.id_prefix);
        let mut cand = base.clone();
        for i in 2.. {
            if self.used_ids.insert(cand.clone()) {
                break;
            }
            cand = format!("{base}-{i}");
        }
        cand
    }

    /// Reserve ids for every named object up front so generated def ids never collide with them.
    fn assign_name_ids(&mut self) {
        if self.opts.object_ids == ObjectIds::Minimal {
            return;
        }
        let mut named: Vec<(NodeId, String)> = Vec::new();
        self.doc.walk(|n| {
            if let Some(name) = &n.name {
                named.push((n.id, sanitize_id(name)));
            }
        });
        for (id, base) in named {
            let cand = self.unique_id(&base);
            self.names.insert(id, cand);
        }
    }
    /// ` id="…"` for a named object, plus ` data-name="…"` with the name itself when the id had to
    /// differ from it (spaces, punctuation, duplicates, the unique prefix).
    /// The id of `n`'s element (with its name when the id isn't it), a sublayer's mark
    /// ([`crate::import::SUBLAYER`]), then the object's own data as `data-*` attributes.
    fn id_attr(&self, n: &Node) -> String {
        if self.anonymous {
            return String::new();
        }
        let mut out = match (self.names.get(&n.id), n.name.as_deref()) {
            (Some(id), Some(name)) if name != id => format!(" id=\"{}\" data-name=\"{}\"", xml_escape(id), xml_escape(name)),
            (Some(id), _) => format!(" id=\"{}\"", xml_escape(id)),
            (None, _) => String::new(),
        };
        // A sublayer is marked as one, so import makes it a sublayer again, not a group.
        if !out.is_empty() && n.is_layer() && self.layer_nest > 1 {
            out.push_str(&format!(" {}=\"sublayer\"", crate::import::SUBLAYER));
        }
        for (k, v) in n.attrs.as_deref().map_or(&[][..], |a| a.data.as_slice()) {
            // Names as XML takes them; ours (`name`, `vc-…`) are written by the export itself.
            let valid = k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && k != "name" && !k.starts_with("vc-");
            if valid && !k.is_empty() {
                out.push_str(&format!(" data-{k}=\"{}\"", xml_escape(v)));
            }
        }
        out
    }
    fn matrix(&self, m: Affine) -> String {
        let c = m.as_coeffs();
        format!("matrix({} {} {} {} {} {})", self.num(c[0]), self.num(c[1]), self.num(c[2]), self.num(c[3]), self.num(c[4]), self.num(c[5]))
    }

    /// Style properties as attributes, an inline style, or a CSS class, per the styling option.
    fn attrs(&mut self, props: &Props) -> String {
        if props.is_empty() {
            return String::new();
        }
        // SVG Tiny has presentation attributes alone.
        let styling = if self.tiny() { Styling::PresentationAttributes } else { self.opts.styling };
        match styling {
            Styling::PresentationAttributes => {
                // CSS-only properties go in a style attribute (SVG Tiny has none: left out).
                let (style, attrs): (Vec<_>, Vec<_>) =
                    props.iter().partition(|(k, _)| matches!(*k, "mix-blend-mode" | "isolation" | "font-feature-settings" | "font-kerning"));
                let mut s: String = attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", xml_escape(v))).collect();
                if !style.is_empty() {
                    if self.tiny() {
                        self.warn("SVG Tiny 1.2 has no style sheets: blend modes, isolation and font features are left out");
                    } else {
                        s.push_str(&format!(" style=\"{}\"", xml_escape(&css::declarations(style))));
                    }
                }
                s
            }
            Styling::InlineStyle => format!(" style=\"{}\"", xml_escape(&css::declarations(props))),
            Styling::StyleEntities => format!(" style=\"&st{};\"", self.class(css::declarations(props))),
            Styling::InternalCss => {
                let i = self.class(css::declarations(props));
                format!(" class=\"{}cls-{i}\"", self.id_prefix)
            }
        }
    }

    /// The 1-based number of a distinct declaration block (a CSS class or a style entity).
    fn class(&mut self, decl: String) -> usize {
        match self.classes.iter().position(|c| *c == decl) {
            Some(i) => i + 1,
            None => {
                self.classes.push(decl);
                self.classes.len()
            }
        }
    }

    /// `<metadata>`: the Dublin Core terms of File Info ([`ExportOptions::metadata`]) and the
    /// native document ([`ExportOptions::preserve_editing`]) with the [`body_hash`] of the markup
    /// around this element.
    fn metadata(&self, out: &mut String, native: Option<(&[u8], String)>) {
        if !self.opts.metadata && native.is_none() {
            return;
        }
        let nl = if self.opts.minify { "" } else { "\n" };
        let mut line = |depth: usize, s: &str| {
            out.push_str(&self.indent(depth));
            out.push_str(s);
            out.push_str(nl);
        };
        line(1, "<metadata>");
        if self.opts.metadata {
            line(2, "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">");
            line(3, "<rdf:Description rdf:about=\"\">");
            for (term, value) in dublin_core(self.doc) {
                line(4, &format!("<dc:{term}>{}</dc:{term}>", xml_escape(&value)));
            }
            line(3, "</rdf:Description>");
            line(2, "</rdf:RDF>");
        }
        if let Some((bytes, hash)) = native {
            line(
                2,
                &format!(
                    "<vectorcraft:document xmlns:vectorcraft=\"{EDITING_NS}\" hash=\"{hash}\"><![CDATA[{}]]></vectorcraft:document>",
                    base64_encode(bytes)
                ),
            );
        }
        line(1, "</metadata>");
    }

    fn path_d(&self, p: &PathData, m: Affine) -> String {
        let mut s = String::new();
        let pt = |p: Point| {
            let p = m * p;
            format!("{} {}", self.num(p.x), self.num(p.y))
        };
        for sp in &p.subpaths {
            let n = sp.anchors.len();
            if n == 0 {
                continue;
            }
            if !s.is_empty() {
                s.push(' ');
            }
            s.push('M');
            s.push_str(&pt(sp.anchors[0].p));
            for i in 0..sp.segment_count() {
                let (a, b) = (&sp.anchors[i], &sp.anchors[(i + 1) % n]);
                if sp.segment_is_line(i) {
                    if sp.closed && i == n - 1 {
                        break;
                    }
                    s.push_str(&format!(" L{}", pt(b.p)));
                } else {
                    s.push_str(&format!(" C{} {} {}", pt(a.h_out), pt(b.h_in), pt(b.p)));
                }
            }
            if sp.closed {
                s.push_str(" Z");
            }
        }
        s
    }

    /// Paint a shape (`d`) with a node's appearance stack. `paths` is the same shape in document
    /// space, for strokes that aren't plain (it may be empty when all are).
    fn shape(&mut self, n: &Node, d: &str, paths: &[&PathData], rule: FillRule, bounds: Option<Rect>) {
        let id = self.id_attr(n);
        let items: Vec<&AppearanceItem> = n
            .appearance
            .items
            .iter()
            .filter(|i| match i {
                AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
                AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
            })
            .collect();
        let mut bp = None;
        let plans: Vec<Option<StrokePlan>> = items
            .iter()
            .map(|i| match i {
                AppearanceItem::Stroke(s) => Some(self.stroke_plan(s, &mut bp, paths)),
                AppearanceItem::Fill(_) => None,
            })
            .collect();
        let fills = items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).count();
        let strokes = items.len() - fills;
        // A stroke's paint box is the bounds grown by half its weight, as on the canvas.
        let paint_bounds = |s: &StrokeLayer| bounds.map(|b| s.paint_bounds(b));
        let simple = fills <= 1
            && strokes <= 1
            && !items.iter().any(|i| has_raster(i.effects()))
            && items.iter().all(|i| i.blend() == BlendMode::Normal)
            && !items.iter().any(|i| freeform(i.paint()).is_some())
            && plans.iter().flatten().all(|p| matches!(p, StrokePlan::Written(Written { shape: WrittenShape::Stroke { .. }, side: None })));
        if simple {
            let mut p = Props::new();
            match items.iter().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None }) {
                Some(f) => self.fill_props(f, rule, bounds, &mut p),
                None => {
                    p.push(("fill", "none".into()));
                    if rule == FillRule::EvenOdd {
                        p.push(("fill-rule", "evenodd".into()));
                    }
                }
            }
            if let Some(s) = items.iter().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None }) {
                self.stroke_props(s, s.width, paint_bounds(s), &mut p);
                if matches!(items.first(), Some(AppearanceItem::Stroke(_))) && fills == 1 {
                    p.push(("paint-order", "stroke".into()));
                }
            }
            p.extend(css::transparency(n));
            let a = self.attrs(&p);
            self.line(&format!("<path{id} d=\"{d}\"{a}/>"));
            return;
        }
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        for (it, plan) in items.into_iter().zip(plans) {
            // The item's own raster effects filter that item alone.
            let filters = self.open_filters(n, it.effects());
            let mut p = Props::new();
            match (it, plan) {
                (AppearanceItem::Fill(f), _) => {
                    let blend = (f.blend != BlendMode::Normal).then(|| ("mix-blend-mode", blend_css(f.blend).to_string()));
                    if let (Some(g), Some(b)) = (freeform(&f.paint), bounds) {
                        p.extend((f.opacity < 1.0).then(|| ("opacity", fmt_num(f.opacity as f64, 3))));
                        p.extend(blend);
                        self.raster_paint(g, b, &[d.to_string()], rule, &p, "");
                    } else {
                        self.fill_props(f, rule, bounds, &mut p);
                        p.extend(blend);
                        let a = self.attrs(&p);
                        self.line(&format!("<path d=\"{d}\"{a}/>"));
                    }
                }
                (AppearanceItem::Stroke(s), Some(StrokePlan::Brush(art))) => {
                    // The brush art takes the stroke's opacity and blend mode as a group.
                    if s.opacity < 1.0 {
                        p.push(("opacity", fmt_num(s.opacity as f64, 3)));
                    }
                    if s.blend != BlendMode::Normal {
                        p.push(("mix-blend-mode", blend_css(s.blend).into()));
                    }
                    let a = self.attrs(&p);
                    self.line(&format!("<g{a}>"));
                    self.depth += 1;
                    for piece in &art {
                        self.node(piece);
                    }
                    self.depth -= 1;
                    self.line("</g>");
                }
                (AppearanceItem::Stroke(s), Some(StrokePlan::Written(w))) => {
                    let side = self.side_attr(w.side, d, rule, w.reach(s, bounds.unwrap_or_default()));
                    let blend = (s.blend != BlendMode::Normal).then(|| ("mix-blend-mode", blend_css(s.blend).to_string()));
                    let opacity = (s.opacity < 1.0).then(|| ("opacity", fmt_num(s.opacity as f64, 3)));
                    match (&w.shape, freeform(&s.paint).zip(paint_bounds(s))) {
                        (WrittenShape::Stroke { width }, Some((g, b))) => {
                            // The stroke's outline clips the gradient's image.
                            let bp = bp.get_or_insert_with(|| doc_path(paths));
                            let outline = stroke::line_outline(bp, s, *width, OUTLINE_TOLERANCE);
                            let clip = self.path_d(&PathData::from_bezpath(&outline), self.xf);
                            p.extend(opacity);
                            p.extend(blend);
                            self.raster_paint(g, b, &[clip], FillRule::NonZero, &p, &side);
                        }
                        (WrittenShape::Fill(outlines), Some((g, b))) if s.path_gradient().is_none() => {
                            let clip: Vec<String> = outlines.iter().map(|o| self.path_d(&PathData::from_bezpath(o), self.xf)).collect();
                            p.extend(opacity);
                            p.extend(blend);
                            self.raster_paint(g, b, &clip, FillRule::NonZero, &p, &side);
                        }
                        (WrittenShape::Stroke { width }, _) => {
                            p.push(("fill", "none".into()));
                            self.stroke_props(s, *width, paint_bounds(s), &mut p);
                            p.extend(blend);
                            let a = self.attrs(&p);
                            self.line(&format!("<path d=\"{d}\"{a}{side}/>"));
                        }
                        (WrittenShape::Fill(outlines), _) if s.path_gradient().is_some() => {
                            // A gradient along or across the stroke: slices clipped to its outlines.
                            if let Some(ws) = bp.as_ref().and_then(|bp| stroke::written_slices(bp, rule, s, outlines)) {
                                p.extend(opacity);
                                p.extend(blend);
                                self.sliced(&ws, &p, &side);
                            }
                        }
                        (WrittenShape::Fill(outlines), _) => {
                            let paint = self.paint(&s.paint, paint_bounds(s));
                            let ds: Vec<String> = outlines.iter().map(|o| self.path_d(&PathData::from_bezpath(o), self.xf)).collect();
                            let opacity = (s.opacity < 1.0).then(|| fmt_num(s.opacity as f64, 3));
                            if let [one] = &ds[..] {
                                p.push(("fill", paint));
                                p.extend(opacity.map(|o| ("fill-opacity", o)));
                                p.extend(blend);
                                let a = self.attrs(&p);
                                self.line(&format!("<path d=\"{one}\"{a}{side}/>"));
                            } else if !ds.is_empty() {
                                // The line and its arrowheads overlap: one group takes the opacity.
                                p.extend(opacity.map(|o| ("opacity", o)));
                                p.extend(blend);
                                let a = self.attrs(&p);
                                self.line(&format!("<g{a}{side}>"));
                                self.depth += 1;
                                let fill = self.attrs(&vec![("fill", paint)]);
                                for one in &ds {
                                    self.line(&format!("<path d=\"{one}\"{fill}/>"));
                                }
                                self.depth -= 1;
                                self.line("</g>");
                            }
                        }
                    }
                }
                (AppearanceItem::Stroke(_), None) => {}
            }
            self.close_filters(filters);
        }
        self.depth -= 1;
        self.line("</g>");
    }

    /// A stroke whose gradient runs along or across it ([`stroke::written_slices`]): a group (with
    /// `props` and the `side` attribute) of its slices, clipped to its outlines.
    fn sliced(&mut self, ws: &stroke::WrittenSlices, props: &Props, side: &str) {
        self.warn("gradients along or across strokes are written as slices of linear gradients");
        let cid = self.fresh_id("clip-path");
        let clip = self.path_d(&PathData::from_bezpath(&ws.clip), self.xf);
        self.def(1, &format!("<clipPath id=\"{cid}\">"));
        self.def(2, &format!("<path d=\"{clip}\"/>"));
        self.def(1, "</clipPath>");
        let a = self.attrs(props);
        self.line(&format!("<g{a}{side}>"));
        self.depth += 1;
        self.line(&format!("<g clip-path=\"url(#{cid})\">"));
        self.depth += 1;
        for (shape, paint) in &ws.slices {
            let fill = self.paint(paint, None);
            let fill = self.attrs(&vec![("fill", fill)]);
            let d = self.path_d(&PathData::from_bezpath(shape), self.xf);
            self.line(&format!("<path d=\"{d}\"{fill}/>"));
        }
        self.depth -= 1;
        self.line("</g>");
        self.depth -= 1;
        self.line("</g>");
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// An object with an opacity mask: `<g mask="url(#…)">` around the unmasked object. The mask
    /// art goes in `<defs>`; no-clip adds a white backdrop and invert a colour-inverting filter.
    fn masked(&mut self, n: &Node, m: &vectorcraft_doc::OpacityMask) {
        if self.tiny() {
            self.warn(TINY_MASKS);
            let mut bare = n.clone();
            bare.mask = None;
            return self.node_body(&bare);
        }
        let mid = self.fresh_id("mask");
        // Mask art is a picture of its own: it takes no part in a knockout group around the object.
        let knockout = std::mem::take(&mut self.knockout);
        let art = self.detached(2, |w| w.node(&m.art));
        self.knockout = knockout;
        let inv = m.invert.then(|| {
            let fid = self.fresh_id("invert");
            self.def(1, &format!("<filter id=\"{fid}\" filterUnits=\"userSpaceOnUse\" color-interpolation-filters=\"sRGB\" {BIG}>"));
            self.def(2, "<feColorMatrix type=\"matrix\" values=\"-1 0 0 0 1 0 -1 0 0 1 0 0 -1 0 1 0 0 0 1 0\"/>");
            self.def(1, "</filter>");
            fid
        });
        // The mask's options, so import restores them (and its art without backdrop and filter).
        let flags: Vec<&str> = [(!m.clip).then_some("noclip"), m.invert.then_some("invert")].into_iter().flatten().collect();
        let data = if flags.is_empty() { String::new() } else { format!(" {MASK_FLAGS}=\"{}\"", flags.join(" ")) };
        self.def(1, &format!("<mask id=\"{mid}\" {MASK} {BIG}{data}>"));
        if let Some(fid) = &inv {
            self.def(1, &format!("<g filter=\"url(#{fid})\">"));
        }
        if !m.clip || m.invert {
            let c = if m.clip { "black" } else { "white" };
            self.def(1, &format!("<rect {BIG} fill=\"{c}\"/>"));
        }
        self.defs.push_str(&art);
        if inv.is_some() {
            self.def(1, "</g>");
        }
        self.def(1, "</mask>");
        self.line(&format!("<g mask=\"url(#{mid})\">"));
        self.depth += 1;
        let mut bare = n.clone();
        bare.mask = None;
        self.node_body(&bare);
        self.depth -= 1;
        self.line("</g>");
    }

    /// What `write` writes to the body, taken out of it (written at `depth`, for `<defs>`).
    fn detached(&mut self, depth: usize, write: impl FnOnce(&mut Self)) -> String {
        let (body, saved) = (std::mem::take(&mut self.body), self.depth);
        self.depth = depth;
        write(self);
        self.depth = saved;
        std::mem::replace(&mut self.body, body)
    }

    /// Props of a group (or layer): a knockout group is isolated, a hidden one not displayed.
    fn group_props(&self, n: &Node) -> Props {
        let mut p = css::transparency(n);
        if !n.isolate && n.knocks_out(self.knockout) {
            p.push(("isolation", "isolate".into()));
        }
        p
    }

    /// The children of group `n`, as the elements of a knockout group when it is one.
    fn group_children(&mut self, n: &Node, children: &[std::sync::Arc<Node>]) {
        let knockout = n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        self.children(children);
        self.knockout = enclosing;
    }

    /// Children of the group being written. SVG has no knockout groups: in one, each element is
    /// drawn through a mask of where the elements above it don't paint (the same look for Normal
    /// blending). The masks nest, so element i sits inside the masks of elements i+1…n.
    fn children(&mut self, children: &[std::sync::Arc<Node>]) {
        if self.knockout && self.tiny() {
            self.warn(TINY_MASKS);
        }
        if !self.knockout || self.tiny() {
            for c in children {
                self.node(c);
            }
            return;
        }
        let elements = Node::knockout_elements(children);
        let Some((first, rest)) = elements.split_first() else { return };
        for c in rest.iter().rev() {
            let mid = self.knockout_mask(c);
            self.line(&format!("<g mask=\"url(#{mid})\">"));
            self.depth += 1;
        }
        self.node(first);
        for c in rest {
            self.depth -= 1;
            self.line("</g>");
            self.node(c);
        }
    }

    /// A mask that is 1 − the knockout shape of `c`: white, then `c` painted black (at full object
    /// opacity without its own mask, unless those define its shape). Returns the mask id.
    fn knockout_mask(&mut self, c: &Node) -> String {
        let filter = match &self.knockout_filter {
            Some(f) => f.clone(),
            None => {
                let f = self.fresh_id("knockout-shape");
                self.def(1, &format!("<filter id=\"{f}\" filterUnits=\"userSpaceOnUse\" {BIG}>"));
                self.def(2, "<feColorMatrix type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\"/>");
                self.def(1, "</filter>");
                self.knockout_filter = Some(f.clone());
                f
            }
        };
        let shape = if c.knockout_shape { c.clone() } else { Node { opacity: 1.0, mask: None, ..c.clone() } };
        let anonymous = std::mem::replace(&mut self.anonymous, true);
        let art = self.detached(3, |w| w.node(&shape));
        self.anonymous = anonymous;
        let mid = self.fresh_id("knockout");
        self.def(1, &format!("<mask id=\"{mid}\" {MASK} {BIG}>"));
        self.def(2, &format!("<rect {BIG} fill=\"white\"/>"));
        self.def(2, &format!("<g filter=\"url(#{filter})\">"));
        self.defs.push_str(&art);
        self.def(2, "</g>");
        self.def(1, "</mask>");
        mid
    }

    /// An object, inside `<a>` when it links to a URL (Attributes panel).
    fn node(&mut self, n: &Node) {
        let layer = usize::from(n.is_layer());
        self.layer_nest += layer;
        match n.url().filter(|_| n.visible && !self.anonymous) {
            Some(url) => {
                self.line(&format!("<a xlink:href=\"{}\">", xml_escape(url)));
                self.depth += 1;
                self.node_body(n);
                self.depth -= 1;
                self.line("</a>");
            }
            None => self.node_body(n),
        }
        self.layer_nest -= layer;
    }

    fn node_body(&mut self, n: &Node) {
        // Hidden layers and objects are left out, unless the options keep them (hidden). Template
        // layers never print.
        if !(n.visible || self.opts.hidden_layers) {
            return;
        }
        if has_raster(&n.appearance.effects) {
            return self.filtered(n);
        }
        if let Some(m) = n.mask.as_deref()
            && !m.disabled
        {
            return self.masked(n, m);
        }
        match &n.kind {
            NodeKind::Layer { template: true, .. } => {}
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                let id = self.id_attr(n);
                let a = self.attrs(&self.group_props(n));
                self.line(&format!("<g{id}{a}>"));
                self.depth += 1;
                self.group_children(n, children);
                self.depth -= 1;
                self.line("</g>");
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                let Some((clip, rest)) = children.split_first() else { return };
                let cid = self.fresh_id("clip-path");
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                // The region every output clips to; with nothing to clip by, an empty clip path
                // hides the clipped art.
                if let Some((bp, rule)) = vectorcraft_effects::clip_outline(clip) {
                    let d = self.path_d(&PathData::from_bezpath(&bp), self.xf);
                    let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
                    self.def(2, &format!("<path d=\"{d}\"{r}/>"));
                }
                self.def(1, "</clipPath>");
                let id = self.id_attr(n);
                let a = self.attrs(&self.group_props(n));
                // The clipping path's fill paints behind the clipped art and its stroke over it,
                // outside the clip (the group then wraps both).
                let paint = clip.clip_paint();
                if paint.stroke.is_some() {
                    self.line(&format!("<g{id}{a}>"));
                    self.depth += 1;
                    self.line(&format!("<g clip-path=\"url(#{cid})\">"));
                } else {
                    self.line(&format!("<g{id} clip-path=\"url(#{cid})\"{a}>"));
                }
                self.depth += 1;
                if let Some(fill) = &paint.fill {
                    self.node(fill);
                }
                self.group_children(n, rest);
                self.depth -= 1;
                self.line("</g>");
                if let Some(stroke) = &paint.stroke {
                    // One element keeps the clipping path's id.
                    let anonymous = self.anonymous;
                    self.anonymous |= paint.fill.is_some();
                    self.node(stroke);
                    self.anonymous = anonymous;
                    self.depth -= 1;
                    self.line("</g>");
                }
            }
            NodeKind::Path { guide: true, .. } => {}
            NodeKind::Path { path, rule, .. } => {
                if path.is_empty() {
                    return;
                }
                let d = self.path_d(path, self.xf);
                self.shape(n, &d, &[path], *rule, n.geometric_bounds());
            }
            NodeKind::Compound { children, rule } => {
                let paths: Vec<&PathData> = children.iter().filter(|c| c.visible).filter_map(|c| c.path_data()).filter(|p| !p.is_empty()).collect();
                if paths.is_empty() {
                    return;
                }
                let d: Vec<String> = paths.iter().map(|p| self.path_d(p, self.xf)).collect();
                self.shape(n, &d.join(" "), &paths, *rule, n.geometric_bounds());
            }
            NodeKind::Text(t) => self.text_node(n, t),
            NodeKind::Image(im) => {
                let id = self.id_attr(n);
                let m = self.matrix(self.xf * im.xf);
                let a = self.attrs(&css::transparency(n));
                if let Some(shared) = self.shared_images.get(&(im.key.clone(), im.width, im.height)) {
                    let shared = shared.clone();
                    self.line(&format!("<use{id} transform=\"{m}\"{a} xlink:href=\"#{shared}\"/>"));
                    return;
                }
                let Some(href) = self.image_href(im) else { return };
                self.line(&format!(
                    "<image{id} width=\"{}\" height=\"{}\" transform=\"{m}\" preserveAspectRatio=\"none\"{a} xlink:href=\"{}\"/>",
                    im.width,
                    im.height,
                    xml_escape(&href)
                ));
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                let doc = self.doc;
                let Some(sym) = doc.symbols.iter().find(|s| s.name == *symbol) else { return };
                if self.symbol_nest > MAX_SYMBOL_NEST {
                    return;
                }
                let id = self.id_attr(n);
                let a = self.attrs(&css::transparency(n));
                if let Some(def) = self.symbol_use(sym, n, *xf) {
                    let m = self.xf * *xf;
                    let tr = if m == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(m)) };
                    self.line(&format!("<use{id} xlink:href=\"#{def}\"{tr}{a}/>"));
                    return;
                }
                // An instance of its own: the (stained) art moved by the instance, its pattern
                // paints and unlinked masks left in place as on the canvas.
                let mut art = vectorcraft_brush::instance_art(&sym.art, n);
                let moved = self.instance_xf * *xf;
                if moved.determinant().abs() > 1e-12 {
                    pin_to_page(&mut art, moved.inverse());
                }
                let saved = (self.xf, self.instance_xf);
                self.xf = saved.0 * *xf;
                self.instance_xf = moved;
                self.line(&format!("<g{id}{a}>"));
                self.depth += 1;
                // The symbol's art is the instance's own picture, outside any knockout around it.
                let knockout = std::mem::take(&mut self.knockout);
                self.symbol_nest += 1;
                self.node(&art);
                self.symbol_nest -= 1;
                self.knockout = knockout;
                self.depth -= 1;
                self.line("</g>");
                (self.xf, self.instance_xf) = saved;
            }
            // Live blends/envelopes/meshes export their evaluated (expanded) form.
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::PlacedDocument(_) => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                self.share_images(&g);
                self.node_body(&g);
            }
        }
    }

    /// A freeform gradient `g` painted over `bounds` (the space [`Self::xf`] maps into user
    /// space), which SVG can't express: an `<image>` of the colour field over its painted box,
    /// sampled as the canvas samples it (at twice the box's size in points) and clipped to
    /// `clip` (outlines in user space filled by `rule`), in a group with `props` and the `side`
    /// attribute. Nothing is written for an empty box.
    fn raster_paint(&mut self, g: &GradientPaint, bounds: Rect, clip: &[String], rule: FillRule, props: &Props, side: &str) {
        use vectorcraft_color::freeform::{grid_size, painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let (cols, rows) = grid_size(b, 2.0 * b.width().max(b.height()));
        let field = g.freeform_on(b).field(spread_scale(b));
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let rgba: Vec<u8> = field.grid(b, cols, rows).flat_map(|([r, g, bl], a)| [q(r), q(g), q(bl), q(a)]).collect();
        let mut png = Vec::new();
        let Some(img) = image::RgbaImage::from_raw(u32::from(cols), u32::from(rows), rgba) else { return };
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            return;
        }
        self.warn("freeform gradients are written as images clipped to their shapes");
        let href = self.blob_href(&ImageBlob::new("image/png", png));
        let cid = self.fresh_id("clip-path");
        let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
        self.def(1, &format!("<clipPath id=\"{cid}\">"));
        for d in clip {
            self.def(2, &format!("<path d=\"{d}\"{r}/>"));
        }
        self.def(1, "</clipPath>");
        let a = self.attrs(props);
        let nested = !side.is_empty();
        if nested {
            self.line(&format!("<g{a}{side}>"));
            self.depth += 1;
            self.line(&format!("<g clip-path=\"url(#{cid})\">"));
        } else {
            self.line(&format!("<g clip-path=\"url(#{cid})\"{a}>"));
        }
        self.depth += 1;
        let (x, y, w, h) = (self.num(b.x0), self.num(b.y0), self.num(b.width()), self.num(b.height()));
        let tr = if self.xf == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(self.xf)) };
        self.line(&format!(
            "<image x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"{tr} preserveAspectRatio=\"none\" xlink:href=\"{}\"/>",
            xml_escape(&href)
        ));
        self.depth -= 1;
        self.line("</g>");
        if nested {
            self.depth -= 1;
            self.line("</g>");
        }
    }
}

/// A shape's paths as one path (document space).
fn doc_path(paths: &[&PathData]) -> BezPath {
    let mut bp = BezPath::new();
    for p in paths {
        bp.extend(p.to_bezpath());
    }
    bp
}

/// How closely stroke outlines that clip an image follow the stroke (points).
const OUTLINE_TOLERANCE: f64 = 0.01;

/// The freeform gradient a paint is, if it is one (SVG has no freeform gradients).
fn freeform(p: &Paint) -> Option<&GradientPaint> {
    match p {
        Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => Some(g),
        _ => None,
    }
}

/// Any visible raster effect (shadow, glow, blur, feather) in `effects`?
fn has_raster(effects: &[vectorcraft_doc::Effect]) -> bool {
    effects.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id))
}
