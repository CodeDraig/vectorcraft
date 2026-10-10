//! Text runs, line segmentation and SVG type emission.

use super::{Affine, Justify, Props, Rect, TextObject};

/// What the `<tspan>` of a run whose properties are `run` sets over its `<text>`'s `base`: the
/// properties that differ, and the initial value of each one `base` sets and the run doesn't.
fn run_diff(base: &Props, run: Props) -> Props {
    let reset: Props =
        base.iter().filter(|(k, _)| !run.iter().any(|(r, _)| r == k)).filter_map(|(k, _)| initial_value(k).map(|v| (*k, v.to_string()))).collect();
    run.into_iter().filter(|kv| !base.contains(kv)).chain(reset).collect()
}

/// The initial value of an inherited property a run may leave unset (decorations can't be undone
/// on a `<tspan>`).
fn initial_value(k: &str) -> Option<&'static str> {
    Some(match k {
        "font-weight" | "font-style" | "font-feature-settings" => "normal",
        "stroke" | "stroke-dasharray" => "none",
        "stroke-width" | "stroke-opacity" => "1",
        "stroke-linecap" => "butt",
        "stroke-linejoin" => "miter",
        "stroke-miterlimit" => "4",
        "stroke-dashoffset" | "letter-spacing" => "0",
        "font-kerning" => "auto",
        _ => return None,
    })
}

/// What the lines of one `<text>` share.
#[derive(Clone)]
struct TextBase {
    /// The `<text>` element's character properties, which each `<tspan>` differs from.
    props: Props,
    /// The text's layout bounds and the map from text space to user space (character paints).
    space: (Rect, Affine),
    /// Lines anchored at their centre or right end (`text-anchor`), positioned once each.
    anchored: bool,
}

/// One laid-out line: its paragraph (index) and its segments.
struct Line {
    para: usize,
    segs: Vec<Segment>,
}

/// Characters of one style on one line, written as one `<tspan>`.
struct Segment {
    /// Index into `TextObject::runs`.
    run: usize,
    /// Where the segment starts (text space; `y` is the baseline).
    x: f64,
    y: f64,
    text: String,
    /// The laid-out width (tracking and justification included).
    advance: f64,
    /// The next segment needs its own position (after a tab, or a word space of a justified line).
    brk: bool,
}

/// The laid-out lines of `t` as segments: a new segment at every style change, after tabs and,
/// on justified lines, after each word space (unless `fewer`). The text comes from the clusters
/// the layout placed: soft hyphens draw nothing, a line broken by hyphenation ends in `-`, all
/// caps are upper case.
fn text_lines(t: &TextObject, lay: &vectorcraft_text::TextLayout, fewer: bool) -> Vec<Line> {
    let plain = t.plain_text();
    let mut lines = Vec::with_capacity(lay.lines.len());
    for line in &lay.lines {
        let para = plain.as_bytes().get(..line.start).map_or(0, |b| b.iter().filter(|&&c| c == b'\n').count());
        let split_words = !fewer && !matches!(t.para_at(para).justify, Justify::Auto | Justify::Left | Justify::Center | Justify::Right);
        let mut segs: Vec<Segment> = Vec::new();
        let mut cluster = None;
        let source = |g: &vectorcraft_text::PositionedGlyph| if g.len == 0 { "-" } else { plain.get(g.byte..g.byte + g.len).unwrap_or("") };
        let mut glyphs = lay.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or_default();
        // Spaces where the line wrapped (it doesn't end its paragraph) don't belong to the line.
        if plain.get(line.end..).is_some_and(|rest| !rest.is_empty() && !rest.starts_with('\n')) {
            while let Some((_, rest)) = glyphs.split_last().filter(|(g, _)| source(g).chars().all(|c| c.is_whitespace() && c != '\t')) {
                glyphs = rest;
            }
        }
        let mut after_inline = false;
        for g in glyphs {
            let Some(run) = t.runs.get(g.run) else { continue };
            if run.inline.is_some() {
                // An inline graphic (written as art): the text after it is placed anew.
                if let Some(last) = segs.last_mut() {
                    last.brk = true;
                }
                after_inline = true;
                cluster = None;
                continue;
            }
            if g.len > 0 && cluster == Some(g.byte) {
                // Another glyph of the same cluster.
                if let Some(last) = segs.last_mut() {
                    last.advance += g.advance;
                }
                continue;
            }
            cluster = Some(g.byte);
            let src: String = source(g).chars().filter(|c| *c != '\u{ad}').collect();
            let piece = if run.style.all_caps { src.to_uppercase() } else { src };
            let brk = piece == "\t" || (split_words && !piece.is_empty() && piece.chars().all(char::is_whitespace));
            let fresh = std::mem::take(&mut after_inline);
            match segs.last_mut() {
                Some(last) if last.run == g.run && !last.brk && !fresh => {
                    last.text.push_str(&piece);
                    last.advance += g.advance;
                    last.brk = brk;
                }
                _ => segs.push(Segment { run: g.run, x: g.origin.x, y: g.origin.y, text: piece, advance: g.advance, brk }),
            }
        }
        segs.retain(|s| !s.text.is_empty());
        if !segs.is_empty() {
            lines.push(Line { para, segs });
        }
    }
    lines
}

/// Where a line of segments starts: its first segment's origin or, `anchor`ed (0.5: centre, 1:
/// right end), that point of its width. `None` for an empty line.
fn line_start(line: &[Segment], anchor: Option<f64>) -> Option<(f64, f64)> {
    let (first, last) = (line.first()?, line.last()?);
    let x = match anchor {
        Some(f) => first.x + f * (last.x + last.advance - first.x),
        None => first.x,
    };
    Some((x, first.y))
}

use super::{
    AppearanceItem, Arc, BTreeSet, BlendMode, CharStyle, FillRule, FontDb, FontFace, FontUse, Node, NodeId, NodeKind, PathData, TextKind, Writer,
    base64_encode, css, css_string, fmt_num, font_descriptor, unicode_range, xml_escape,
};

impl Writer<'_> {
    /// The `<text>`/`<tspan>` properties of a character style. `space`: the text's layout bounds
    /// and the map from text space to user space, which character gradients resolve against
    /// (as on the canvas).
    fn char_props(&mut self, st: &CharStyle, space: (Rect, Affine)) -> Props {
        let decimals = self.opts.decimals;
        let len = |v: f64| fmt_num(v, decimals);
        let mut p = css::font_props(st, &len);
        // Character paints resolve in the text's space (`to_user` maps it into user space).
        let (bounds, to_user) = space;
        let saved = std::mem::replace(&mut self.xf, to_user);
        let fill = self.paint(&st.fill, Some(bounds));
        p.push(("fill", fill));
        if st.has_stroke() {
            // Weight, cap, join, miter limit and dashes as for object strokes; a gradient spans
            // the text grown by half the weight, as on the canvas.
            let layer = st.stroke_layer();
            self.stroke_props(&layer, layer.width, Some(layer.paint_bounds(bounds)), &mut p);
        }
        self.xf = saved;
        p.extend(css::type_props(st, &len));
        p
    }

    /// Type: its characters (live text, or glyph outlines when text is exported as outlines) and,
    /// as the canvas paints them, the object's own fills and strokes on the glyph outlines: those
    /// below the Characters row under the characters, the others over them.
    ///
    /// Inline graphics ([`vectorcraft_doc::TextRun::inline`]) are written after the characters as
    /// instances of their symbols (a `<use>` of the symbol's def where it can be shared), all in
    /// one group carrying the object's transparency. The live text skips their characters and
    /// places the text after each one; type on a path with inline graphics is written as outlines
    /// (a `<textPath>` couldn't leave room for the art).
    pub(super) fn text_node(&mut self, n: &Node, t: &TextObject) {
        let doc = self.doc;
        let resolved = doc.inline_resolved(t);
        let t = &*resolved;
        let inline = t.runs.iter().any(|r| r.inline.is_some());
        let outlined = self.opts.outline_text || t.vertical || (inline && matches!(t.kind, TextKind::OnPath { .. }));
        let chars = |w: &mut Self, n: &Node| if outlined { w.text_outlines(n, t) } else { w.text(n, t) };
        let painted = n.appearance.items.iter().any(|i| i.visible() && !i.paint().is_none());
        if !painted && !inline {
            return chars(self, n);
        }
        let id = self.id_attr(n);
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let pd = PathData::from_bezpath(&lay.to_bezpath()).transformed(t.xf);
        let d = self.path_d(&pd, self.xf);
        let tb = Some(t.xf.transform_rect_bbox(lay.bounds));
        let (below, above) = n.appearance.split_contents();
        let glyphs = |w: &mut Self, items: &[AppearanceItem]| {
            if !items.is_empty() {
                let ap = vectorcraft_doc::Appearance { items: items.to_vec(), ..Default::default() };
                w.shape(&Node::path(NodeId(u64::MAX), pd.clone(), ap), &d, &[&pd], FillRule::NonZero, tb);
            }
        };
        glyphs(self, below);
        // The group carries the id and the object's transparency.
        let bare = Node { opacity: 1.0, blend: BlendMode::Normal, isolate: false, ..n.clone() };
        let anonymous = std::mem::replace(&mut self.anonymous, true);
        chars(self, &bare);
        for ig in &lay.inlines {
            let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
            let inst = Node::new(
                NodeId(u64::MAX),
                NodeKind::SymbolInstance { symbol: art.symbol.clone(), xf: t.xf * ig.xf * doc.symbol_natural_xf(&art.symbol) },
            );
            self.node(&inst);
        }
        glyphs(self, above);
        self.anonymous = anonymous;
        self.depth -= 1;
        self.line("</g>");
    }

    /// Text as glyph outlines: one compound path per run, painted like the run (gradients span
    /// the whole text, as on the canvas).
    fn text_outlines(&mut self, n: &Node, t: &TextObject) {
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let mut runs: Vec<(usize, kurbo::BezPath)> = vec![];
        for g in &lay.glyphs {
            match runs.last_mut() {
                Some((r, bp)) if *r == g.run => bp.extend(g.outline.iter()),
                _ => runs.push((g.run, g.outline.clone())),
            }
        }
        let id = self.id_attr(n);
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        // The glyphs stay in text space; the text's transform joins the document's meanwhile.
        let saved = self.xf;
        self.xf = saved * t.xf;
        for (r, bp) in runs {
            let Some(run) = t.runs.get(r) else { continue };
            let pd = PathData::from_bezpath(&bp);
            // An unnamed id: the pieces carry no id attribute (the group has it).
            let glyphs = Node::path(NodeId(u64::MAX), pd.clone(), run.style.appearance());
            let d = self.path_d(&pd, self.xf);
            self.shape(&glyphs, &d, &[&pd], FillRule::NonZero, Some(lay.bounds));
        }
        self.xf = saved;
        self.depth -= 1;
        self.line("</g>");
    }

    /// Point and area type, every laid-out line at its own position: wraps, indents, tabs and
    /// spacing come out as on the canvas. Centred and right-aligned lines are anchored
    /// (`text-anchor`) at their centre or right end, so they stay aligned in a viewer whose font
    /// differs; other lines are placed where each style run (and, justified, each word) starts.
    fn text(&mut self, n: &Node, t: &TextObject) {
        // Live SVG text is written in logical order from left-aligned pieces: bidirectional text
        // (and right-to-left paragraphs) keep their look as outlines until it is written with
        // `direction`/`unicode-bidi`.
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        if lay.glyphs.iter().any(|g| g.rtl) || lay.lines.iter().any(|l| l.rtl) {
            self.warn("bidirectional text is outlined to preserve shaping and visual order");
            return self.text_outlines(n, t);
        }
        if let TextKind::OnPath { path, start, end } = &t.kind {
            // `<textPath>` has a start offset only: an end bracket, Align to Path and Spacing
            // keep their look as outlines.
            if end.is_some() || t.path_align != vectorcraft_doc::PathAlign::Baseline || t.path_spacing != 0.0 {
                self.warn("type on a path with an end bracket, Align to Path or Spacing is outlined to keep its look");
                return self.text_outlines(n, t);
            }
            return self.text_on_path(n, t, path, *start);
        }
        self.note_fonts(t, &lay);
        let lines = text_lines(t, &lay, self.opts.fewer_tspans);
        // Each line is anchored as its paragraph is aligned. A tab starts a new chunk at its
        // stop, and so does the text after an inline graphic: lines with tabs, and text with
        // inline graphics, are never anchored.
        let has_inline = t.runs.iter().any(|r| r.inline.is_some());
        let anchors: Vec<Option<(&str, f64)>> = lines
            .iter()
            .map(|l| {
                match t.para_at(l.para).justify {
                    Justify::Center => Some(("middle", 0.5)),
                    Justify::Right => Some(("end", 1.0)),
                    _ => None,
                }
                .filter(|_| !has_inline && !l.segs.iter().any(|s| s.brk))
            })
            .collect();
        // Every line anchored alike (the text's only paragraph style, typically): the <text>
        // carries the anchor. Otherwise each anchored line is a <tspan> carrying its own.
        let uniform = anchors.first().copied().filter(|a| a.is_some() && anchors.iter().all(|b| b == a)).flatten();
        // The <text> element's user space is text space.
        let space = (lay.bounds, Affine::IDENTITY);
        let base = TextBase { props: self.char_props(&t.first_style(), space), space, anchored: uniform.is_some() };
        let mut props = base.props.clone();
        if let Some((a, _)) = uniform {
            props.push(("text-anchor", a.into()));
        }
        props.extend(css::transparency(n));
        let id = self.id_attr(n);
        let a = self.attrs(&props);
        let m = self.xf * t.xf;
        let tr = if m == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(m)) };
        let starts: Vec<Option<(f64, f64)>> = lines.iter().zip(&anchors).map(|(l, a)| line_start(&l.segs, a.map(|a| a.1))).collect();
        // The `<text>` carries the first line's position too, so readers that place text by its
        // own x/y start where the first line does.
        let at = match starts.iter().flatten().next() {
            Some((x, y)) => format!(" x=\"{}\" y=\"{}\"", self.num(*x), self.num(*y)),
            None => String::new(),
        };
        let mut s = format!("<text{id}{tr}{at} xml:space=\"preserve\"{a}>");
        for ((line, start), anchor) in lines.iter().zip(starts).zip(&anchors) {
            let Some(start) = start else { continue };
            match anchor.filter(|_| uniform.is_none()) {
                // A line anchored on its own: one positioned <tspan> with the anchor around it.
                Some((a, _)) => {
                    s.push_str(&format!("<tspan x=\"{}\" y=\"{}\" text-anchor=\"{a}\">", self.num(start.0), self.num(start.1)));
                    let inner = TextBase { anchored: true, ..base.clone() };
                    self.text_line(&mut s, t, &line.segs, start, &inner, false);
                    s.push_str("</tspan>");
                }
                None => self.text_line(&mut s, t, &line.segs, start, &base, true),
            }
        }
        s.push_str("</text>");
        self.line(&s);
    }

    /// One line of laid-out text starting at `start`. Default: a positioned `<tspan>` per segment
    /// (only the first of an anchored line). Fewer tspans: one positioned `<tspan>` per line with
    /// the style changes nested in it, positioned again only after tabs and justified word spaces.
    /// Baseline shifts are relative (`dy`), undone by the next segment that isn't shifted.
    /// `positioned`: false when the caller already positioned the line (an outer `<tspan>`).
    fn text_line(&mut self, s: &mut String, t: &TextObject, line: &[Segment], start: (f64, f64), base: &TextBase, positioned: bool) {
        let fewer = self.opts.fewer_tspans && positioned;
        if fewer {
            s.push_str(&format!("<tspan x=\"{}\" y=\"{}\">", self.num(start.0), self.num(start.1)));
        }
        let mut place = !fewer && positioned;
        let mut first = true;
        // The baseline shift the current text position carries.
        let mut shift = 0.0;
        for seg in line {
            let Some(st) = t.runs.get(seg.run).map(|r| &r.style) else { continue };
            let mut attrs = String::new();
            if place {
                let (x, y) = if first { start } else { (seg.x, seg.y) };
                attrs.push_str(&format!(" x=\"{}\" y=\"{}\"", self.num(x), self.num(y)));
                shift = 0.0;
            }
            first = false;
            if st.baseline_shift != shift {
                attrs.push_str(&format!(" dy=\"{}\"", self.num(shift - st.baseline_shift)));
                shift = st.baseline_shift;
            }
            // After a tab or a justified word space the next segment is placed.
            place = seg.brk || (!fewer && !base.anchored && positioned);
            if (st.h_scale - st.v_scale).abs() > 1e-9 {
                attrs.push_str(&format!(" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"", self.num(seg.advance)));
            }
            if st.rotation != 0.0 {
                attrs.push_str(&format!(" rotate=\"{}\"", self.num(-st.rotation)));
            }
            let diff = run_diff(&base.props, self.char_props(st, base.space));
            attrs.push_str(&self.attrs(&diff));
            let text = xml_escape(&seg.text);
            if attrs.is_empty() && fewer {
                s.push_str(&text);
            } else {
                s.push_str(&format!("<tspan{attrs}>{text}</tspan>"));
            }
        }
        if fewer {
            s.push_str("</tspan>");
        }
    }

    /// Record the characters of laid-out type `t` under the faces that supply them, for
    /// [`Self::font_faces`] (when fonts are embedded).
    fn note_fonts(&mut self, t: &TextObject, lay: &vectorcraft_text::TextLayout) {
        if !self.opts.embed_fonts || self.opts.outline_text {
            return;
        }
        if self.tiny() {
            return self.warn("SVG Tiny 1.2 has no web fonts: fonts are not embedded");
        }
        let db = FontDb::global();
        let plain = t.plain_text();
        let mut face: Option<Arc<FontFace>> = None;
        for g in &lay.glyphs {
            let Some(st) = t.runs.get(g.run).map(|r| &r.style) else { continue };
            if face.as_ref().is_none_or(|f| f.id() != g.font_id) {
                face = db.face_by_id(g.font_id);
            }
            let Some(f) = &face else { continue };
            let (weight, italic) = font_descriptor(st);
            let at = self.fonts.iter().position(|u| u.face.id() == f.id() && (u.weight, u.italic) == (weight, italic) && u.family == st.font_family);
            let at = at.unwrap_or_else(|| {
                let family = st.font_family.clone();
                self.fonts.push(FontUse { family, weight, italic, face: f.clone(), chars: BTreeSet::new() });
                self.fonts.len() - 1
            });
            let Some(chars) = self.fonts.get_mut(at).map(|u| &mut u.chars) else { continue };
            // As written: a hyphen where a line breaks, capitals for All Caps.
            let src = if g.len == 0 { "-" } else { plain.get(g.byte..g.byte + g.len).unwrap_or("") };
            for c in src.chars().filter(|c| !c.is_control() && *c != '\u{ad}') {
                if st.all_caps {
                    chars.extend(c.to_uppercase());
                } else {
                    chars.insert(c);
                }
            }
        }
    }

    /// The `@font-face` rules of the faces type uses ([`ExportOptions::embed_fonts`]): each face
    /// subset to its characters (whole when its licence forbids subsetting, left out with a
    /// warning when it forbids embedding). Faces sharing a description cover their own
    /// characters (`unicode-range`).
    pub(super) fn font_faces(&mut self) -> Vec<String> {
        let uses = std::mem::take(&mut self.fonts);
        let mut rules = Vec::new();
        for u in &uses {
            let chars: Vec<char> = u.chars.iter().copied().collect();
            let name = format!("{} {}", u.face.family, u.face.style);
            let Some(font) = u.face.embed(&chars) else {
                self.warn(&format!("the licence of the font {name} doesn't allow embedding: viewers show its type in their own fonts"));
                continue;
            };
            if !font.subset {
                self.warn(&format!("the font {name} is embedded whole: its licence doesn't allow subsetting"));
            }
            let shared = uses.iter().filter(|o| (&o.family, o.weight, o.italic) == (&u.family, u.weight, u.italic)).count() > 1;
            let range = if shared { format!(";unicode-range:{}", unicode_range(&u.chars)) } else { String::new() };
            let (mime, format) = if font.cff { ("font/otf", "opentype") } else { ("font/ttf", "truetype") };
            rules.push(format!(
                "@font-face{{font-family:{};font-weight:{};font-style:{};src:url(data:{mime};base64,{}) format(\"{format}\"){range}}}",
                css_string(&u.family),
                u.weight,
                if u.italic { "italic" } else { "normal" },
                base64_encode(&font.data)
            ));
        }
        rules
    }

    /// Type on a path: a `<textPath>` along the path (a def).
    fn text_on_path(&mut self, n: &Node, t: &TextObject, path: &PathData, start: f64) {
        let id = self.id_attr(n);
        let pid = self.fresh_id("text-path");
        let d = self.path_d(path, self.xf * t.xf);
        self.def(1, &format!("<path id=\"{pid}\" d=\"{d}\"/>"));
        // The <text> element's user space is the document's; gradients span the laid-out text.
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        self.note_fonts(t, &lay);
        let space = (lay.bounds, self.xf * t.xf);
        let base = self.char_props(&t.first_style(), space);
        let mut props = base.clone();
        match t.para_at(0).justify {
            Justify::Center | Justify::JustifyCenter => props.push(("text-anchor", "middle".into())),
            Justify::Right | Justify::JustifyRight => props.push(("text-anchor", "end".into())),
            _ => {}
        }
        props.extend(css::transparency(n));
        let a = self.attrs(&props);
        let offset = fmt_num(start * 100.0, 3);
        let mut s = format!("<text{id} xml:space=\"preserve\"{a}><textPath xlink:href=\"#{pid}\" startOffset=\"{offset}%\">");
        for run in &t.runs {
            let diff = run_diff(&base, self.char_props(&run.style, space));
            let a = self.attrs(&diff);
            s.push_str(&format!("<tspan{a}>{}</tspan>", xml_escape(&run.text.replace('\n', " "))));
        }
        s.push_str("</textPath></text>");
        self.line(&s);
    }
}
