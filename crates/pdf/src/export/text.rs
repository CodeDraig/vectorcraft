//! PDF font subsetting and shaped text emission.

use super::{Affine, Shape};

/// Glyphs written together as real text: the same font at the same size along one baseline.
struct GlyphLine {
    face: u32,
    font: krilla::text::Font,
    size: f64,
    /// The glyph space (krilla's, y down, at `size`) → text space, at the line's start.
    frame: Affine,
    /// Glyph id, pen position and advance along the line (in glyph space), and the characters it
    /// stands for (bytes of the plain text).
    glyphs: Vec<(u32, f64, f64, std::ops::Range<usize>)>,
}

use super::{
    AppearanceItem, BezPath, BlendMode, Cow, Embedding, Exporter, Fill, FillRule, GlyphId, KrillaGlyph, MAX_INLINE_DEPTH, Node, NormalizedF32, Paint,
    Path, Rect, Surface, TextLayout, TextObject, norm, to_path, xf,
};

impl Exporter<'_> {
    ///
    /// Inline graphics ([`vectorcraft_doc::TextRun::inline`]) are drawn as their symbols' art
    /// (vector paths, like symbol instances) where the layout placed them; their characters
    /// write no glyph.
    pub(super) fn text(&mut self, s: &mut Surface, n: &Node, t: &TextObject, page: Rect) {
        let doc = self.doc;
        let resolved = doc.inline_resolved(t);
        let t = &*resolved;
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let tb = t.xf.transform_rect_bbox(layout.bounds);
        // The object's own fills and strokes paint the whole outline: those below the Characters
        // row under the characters, the others over them.
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = layout.to_bezpath();
            all.apply_affine(t.xf);
            to_path(&all).map(|p| (all, p))
        });
        let all = all.flatten();
        if let Some((bp, path)) = &all {
            self.text_items(s, below, bp, path, page, tb);
        }
        s.push_transform(&xf(t.xf));
        // Real text: the characters' fills as text in embedded fonts.
        let plain = (!self.outline_text).then(|| t.plain_text());
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
            }
            let fill = &run.style.fill;
            self.overprinting(s, run.style.overprint_fill, fill, |ex, s| {
                // The outlines left to fill as paths: all of them, or those real text leaves.
                let outlines = match plain.as_deref().filter(|_| !fill.is_none() && !ex.area_paint(fill)) {
                    Some(text) => Cow::Owned(ex.glyph_text(s, &layout, i, text, fill)),
                    None => Cow::Borrowed(&bp),
                };
                let opaque = (1.0, BlendMode::Normal);
                if let Some(path) = to_path(&outlines)
                    && !ex.area(s, &path, krilla::paint::FillRule::NonZero, fill, opaque, layout.bounds, outlines.bounding_box())
                    && let Some(paint) = ex.paint(fill, layout.bounds)
                {
                    s.set_stroke(None);
                    s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
                    s.draw_path(&path);
                }
            });
            if run.style.has_stroke()
                && let Some(path) = to_path(&bp)
            {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                let st = run.style.stroke_layer();
                self.overprinting(s, st.overprint, &st.paint, |ex, s| ex.stroke(s, &bp, &path, FillRule::NonZero, &st, page, layout.bounds));
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
        s.pop();
        if self.inline_depth < MAX_INLINE_DEPTH {
            self.inline_depth += 1;
            for ig in &layout.inlines {
                let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
                let Some(sym) = doc.symbols.iter().find(|x| x.name == art.symbol) else { continue };
                let mut node = (*sym.art).clone();
                // Strokes scale with the art, as on the canvas.
                node.transform(t.xf * ig.xf * doc.symbol_natural_xf(&art.symbol), true);
                self.node(s, &node, page, true);
            }
            self.inline_depth -= 1;
        }
        if let Some((bp, path)) = &all {
            self.text_items(s, above, bp, path, page, tb);
        }
    }

    /// Some of a type object's own fills and strokes (`items`) on its glyph outlines `bp` (`path`;
    /// `tb`: their bounds). Strokes take every stroke option, as on paths.
    #[allow(clippy::too_many_arguments)]
    fn text_items(&mut self, s: &mut Surface, items: &[AppearanceItem], bp: &BezPath, path: &Path, page: Rect, tb: Rect) {
        for item in items {
            match item {
                AppearanceItem::Fill(fl) if fl.visible => self.overprinting(s, fl.overprint, &fl.paint, |ex, s| {
                    if !ex.area(s, path, krilla::paint::FillRule::NonZero, &fl.paint, (fl.opacity, fl.blend), tb, bp.bounding_box())
                        && let Some(paint) = ex.paint(&fl.paint, tb)
                    {
                        s.set_stroke(None);
                        s.set_fill(Some(Fill { paint, opacity: norm(fl.opacity), rule: krilla::paint::FillRule::NonZero }));
                        s.draw_path(path);
                    }
                }),
                AppearanceItem::Stroke(st) if st.visible && st.width > 0.0 => {
                    self.overprinting(s, st.overprint, &st.paint, |ex, s| ex.stroke(s, bp, path, FillRule::NonZero, st, page, tb))
                }
                _ => {}
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
    }

    /// Fill the glyphs of run `run` of `layout` with `fill` as text in their fonts, embedded (in
    /// text space; `text` is the plain text, whose characters the glyphs stand for, which makes
    /// the text selectable and searchable) → the outlines of the glyphs left to draw as paths:
    /// those of fonts that can't be embedded, and missing glyphs. Glyphs sharing a font, a frame
    /// (size, scale, rotation) and a baseline are written together.
    fn glyph_text(&mut self, s: &mut Surface, layout: &TextLayout, run: usize, text: &str, fill: &Paint) -> BezPath {
        let mut rest = BezPath::new();
        let mut line: Option<GlyphLine> = None;
        for g in layout.glyphs.iter().filter(|g| g.run == run) {
            let range = g.byte..g.byte + g.len;
            let ch = text.get(range.clone()).and_then(|t| t.chars().next());
            // Spaces are written too (text extraction needs them); control characters and soft
            // hyphens draw nothing.
            let blank = g.outline.elements().is_empty();
            if blank && !ch.is_some_and(|c| c.is_whitespace() && !c.is_control()) {
                continue;
            }
            // The glyph at the font size of its vertical scale: `frame` maps that size's glyph
            // space (y down, krilla's) to text space.
            let [a, b, c, d, ..] = g.xf.as_coeffs();
            let (sx, sy) = (a.hypot(b), c.hypot(d));
            let font = if g.gid != 0 && ch.is_some() { self.font(g.font_id) } else { None };
            let det = g.xf.determinant();
            let Some((font, upem)) = font.filter(|(_, upem)| det.is_finite() && det.abs() > 1e-9 && (1e-3..=1e5).contains(&(upem * sy))) else {
                rest.extend(g.outline.iter());
                continue;
            };
            let size = upem * sy;
            let frame = g.xf * Affine::scale(1.0 / sy);
            // Its pen position along the frame's baseline, when it continues the line.
            let x = line.as_ref().filter(|l| l.face == g.font_id && (l.size - size).abs() < 1e-6).and_then(|l| {
                let [a, b, c, d, e, f] = (l.frame.inverse() * frame).as_coeffs();
                let same = (a - 1.0).abs() < 1e-6 && b.abs() < 1e-6 && c.abs() < 1e-6 && (d - 1.0).abs() < 1e-6 && f.abs() < 1e-4;
                same.then_some(e)
            });
            let x = match x {
                Some(x) => x,
                None => {
                    if let Some(l) = line.take() {
                        self.draw_glyph_line(s, l, text, fill, layout.bounds);
                    }
                    line = Some(GlyphLine { face: g.font_id, font, size, frame, glyphs: vec![] });
                    0.0
                }
            };
            // The glyph's advance in frame units (the last one's ends the line).
            let advance = g.advance * sy / sx;
            if let Some(l) = line.as_mut() {
                l.glyphs.push((g.gid, x, advance, range));
            }
        }
        if let Some(l) = line {
            self.draw_glyph_line(s, l, text, fill, layout.bounds);
        }
        rest
    }

    /// Draw glyphs `line` (of `text`), filled with `fill` (whose space is text space, where
    /// `bounds` is).
    fn draw_glyph_line(&mut self, s: &mut Surface, line: GlyphLine, text: &str, fill: &Paint, bounds: Rect) {
        let Some(paint) = self.paint_in(fill, bounds, line.frame.inverse()) else { return };
        let size = line.size;
        let glyphs: Vec<KrillaGlyph> = line
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, (gid, x, advance, range))| {
                let next = line.glyphs.get(i + 1).map_or(x + advance, |n| n.1);
                KrillaGlyph::new(GlyphId::new(*gid), ((next - x) / size) as f32, 0.0, 0.0, 0.0, range.clone(), None)
            })
            .collect();
        s.push_transform(&xf(line.frame));
        s.set_stroke(None);
        s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
        s.draw_glyphs(krilla::geom::Point::from_xy(0.0, 0.0), &glyphs, line.font, text, size as f32, false);
        s.pop();
    }

    /// The font of face `id` for real text and its units per em; `None` (with a warning) when its
    /// licence doesn't allow embedding it as a subset, or it can't be read.
    fn font(&mut self, id: u32) -> Option<(krilla::text::Font, f64)> {
        if let Some(f) = self.fonts.get(&id) {
            return f.clone();
        }
        let Some(face) = vectorcraft_text::FontDb::global().face_by_id(id) else {
            self.fonts.insert(id, None);
            return None;
        };
        let name = format!("{} {}", face.family, face.style);
        let font = match face.embedding() {
            Embedding::Subset => {
                // A named instance of a variable font is embedded instanced (its own outlines and
                // widths, not the default instance's).
                let coords: Vec<(krilla::text::Tag, f32)> = face.variations().iter().map(|(t, v)| (krilla::text::Tag::new(t), *v)).collect();
                let font =
                    krilla::text::Font::new_variable(face.file_data().to_vec().into(), face.face_index(), &coords).map(|f| (f, face.units_per_em()));
                if font.is_none() {
                    self.warn(format!("the font “{name}” can't be embedded: its text is exported as outlines"));
                }
                font
            }
            Embedding::Whole => {
                self.warn(format!(
                    "the font “{name}” may only be embedded whole, and fonts are embedded as subsets: its text is exported as outlines"
                ));
                None
            }
            Embedding::Forbidden => {
                self.warn(format!("the licence of the font “{name}” doesn't allow embedding: its text is exported as outlines"));
                None
            }
        };
        self.fonts.insert(id, font.clone());
        font
    }
}
