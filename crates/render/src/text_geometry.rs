//! Glyph outline caching, inline art and styled text rendering.

use super::{Affine, BezPath, Rect, Shape, TextObject, peniko};

/// Glyph outlines grouped by run, plus the whole text as one path.
pub(super) struct TextGeom {
    pub(super) runs: Vec<BezPath>,
    pub(super) all: BezPath,
    pub(super) bounds: Rect,
    /// The boxes of characters whose font is missing (drawn in a substitute), and of glyphs the
    /// font lacks (drawn from a fallback font): Document Setup's substitution highlights.
    pub(super) substituted_fonts: BezPath,
    pub(super) substituted_glyphs: BezPath,
    /// Inline graphics placed by the layout.
    pub(super) inlines: Vec<vectorcraft_text::InlineGlyph>,
}

fn text_geom(t: &TextObject) -> TextGeom {
    text_geom_snapped(t, None)
}

/// Glyph geometry of `t`; `snap` (text space → device pixels) puts the glyphs on whole pixels.
pub(super) fn text_geom_snapped(t: &TextObject, snap: Option<Affine>) -> TextGeom {
    let db = vectorcraft_text::FontDb::global();
    let mut layout = vectorcraft_text::layout(db, t);
    if let Some(xf) = snap {
        layout.snap_to_pixels(xf);
    }
    let mut runs = vec![BezPath::new(); t.runs.len()];
    let mut all = BezPath::new();
    // Per run: is its family missing, and the face its glyphs should come from.
    let faces: Vec<(bool, Option<u32>)> = t
        .runs
        .iter()
        .map(|r| match db.resolve(&r.style.font_family, &r.style.font_style) {
            // The version the type names, when it's installed (see `FontDb::face_version`).
            Some((f, m)) => {
                let face = db.face_version(&r.style.font_family, &r.style.font_style, r.style.font_version.as_deref()).unwrap_or(f);
                (m == vectorcraft_text::FontMatch::Missing, Some(face.id()))
            }
            None => (true, None),
        })
        .collect();
    let (mut substituted_fonts, mut substituted_glyphs) = (BezPath::new(), BezPath::new());
    for g in &layout.glyphs {
        if let Some(r) = runs.get_mut(g.run) {
            r.extend(g.outline.iter());
        }
        all.extend(g.outline.iter());
        let Some(&(missing, face)) = faces.get(g.run) else { continue };
        let target = if missing {
            &mut substituted_fonts
        } else if face.is_some_and(|f| f != g.font_id) {
            &mut substituted_glyphs
        } else {
            continue;
        };
        let Some(line) = layout.lines.get(g.line) else { continue };
        let mut cell = Rect::new(g.origin.x, g.origin.y - line.ascent, g.origin.x + g.advance, g.origin.y + line.descent).to_path(0.1);
        cell.apply_affine(Affine::rotate_about(g.angle, g.origin));
        target.extend(cell.iter());
    }
    TextGeom { runs, all, bounds: layout.bounds, substituted_fonts, substituted_glyphs, inlines: layout.inlines }
}

/// Document Setup's highlight behind substituted fonts and glyphs (screen only).
const SUBSTITUTED: peniko::Color = peniko::Color::from_rgba8(255, 120, 190, 110);

use super::{
    AppearanceItem, Arc, BlendMode, FillRule, Frame, MAX_INLINE_DEPTH, MISSING_INLINE_LOGGED, Node, RenderContext, Renderer, blend_mode, kurbo,
    paint, proof,
};

impl Renderer {
    pub(super) fn draw_text(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject) {
        let t = &*f.doc.inline_resolved(t);
        let g = text_geom_snapped(t, f.text_snap(t));
        self.draw_text_geom(ctx, f, n, t, &g);
    }

    /// Draw the inline graphics of type `t` (laid out in `g`): each symbol's art through the
    /// normal node path, placed by the layout (inside the text's own transparency group, so its
    /// opacity and blend mode apply). A missing symbol draws nothing.
    pub(super) fn draw_inlines(&mut self, ctx: &mut RenderContext, f: &Frame, t: &TextObject, g: &TextGeom) {
        if g.inlines.is_empty() && !t.runs.iter().any(|r| r.inline.is_some()) {
            return;
        }
        if self.inline_depth >= MAX_INLINE_DEPTH {
            return;
        }
        for r in t.runs.iter().filter_map(|r| r.inline.as_ref()) {
            if !f.doc.symbols.iter().any(|s| s.name == r.symbol) && !MISSING_INLINE_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!(target: "vectorcraft_render", "inline graphic shows symbol {:?}, which the document doesn't have: it draws nothing", r.symbol);
            }
        }
        self.inline_depth += 1;
        for ig in &g.inlines {
            let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
            let Some(sym) = f.doc.symbols.iter().find(|s| s.name == art.symbol) else { continue };
            let mut node = (*sym.art).clone();
            // Strokes scale with the art (it is sized to the type, as in the SVG `<use>`).
            node.transform(t.xf * ig.xf * f.doc.symbol_natural_xf(&art.symbol), true);
            self.draw_node(ctx, f, &node, true);
        }
        self.inline_depth -= 1;
    }

    /// Cached glyph geometry for a text node (keyed by Arc identity like paths).
    pub(super) fn text_geom_of(&mut self, a: &Arc<Node>, t: &TextObject) -> Arc<TextGeom> {
        let fonts = vectorcraft_text::FontDb::global().generation();
        if fonts != self.text_fonts {
            self.texts.clear();
            self.text_fonts = fonts;
        }
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, g)) = self.texts.get(&key)
            && Arc::ptr_eq(node, a)
        {
            return g.clone();
        }
        let g = Arc::new(text_geom(t));
        if self.texts.len() > 4096 {
            self.texts.clear();
        }
        self.texts.insert(key, (a.clone(), g.clone()));
        g
    }

    pub(super) fn draw_text_geom(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject, g: &TextGeom) {
        let xf = f.view * t.xf;
        if f.opts.outline {
            let mut p = g.all.clone();
            p.apply_affine(xf);
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_stroke(kurbo::Stroke::new(1.0));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.stroke_path(&p);
            self.draw_inlines(ctx, f, t, g);
            return;
        }
        let tb = t.xf.transform_rect_bbox(g.bounds);
        // Overprinting characters multiply per draw, like overprinting fills and strokes
        // ([`proof`]); the object blend mode is Normal here or applied by an enclosing layer.
        let overprints = proof::overprints(f.opts);
        let overprint = |ctx: &mut RenderContext, on: bool| {
            if overprints {
                ctx.set_blend_mode(if on { blend_mode(vectorcraft_color::BlendMode::Multiply) } else { BlendMode::default() });
            }
        };
        // The object's own fills and strokes paint the whole outline: those below the Characters
        // row under the characters' own fill and stroke, the others over them.
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = g.all.clone();
            all.apply_affine(t.xf);
            all
        });
        if let Some(all) = &all {
            self.draw_text_items(ctx, f, n, below, all, tb);
        }
        if f.opts.highlight_substitutions {
            let setup = &f.doc.setup;
            for (on, path) in [(setup.highlight_substituted_fonts, &g.substituted_fonts), (setup.highlight_substituted_glyphs, &g.substituted_glyphs)]
            {
                if on && !path.elements().is_empty() {
                    ctx.set_transform(xf);
                    ctx.set_fill_rule(peniko::Fill::NonZero);
                    ctx.set_paint(SUBSTITUTED);
                    ctx.fill_path(path);
                }
            }
        }
        for (i, run) in t.runs.iter().enumerate() {
            let Some(path) = g.runs.get(i) else { continue };
            if path.elements().is_empty() {
                continue;
            }
            ctx.set_transform(xf);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            if paint::set_paint(ctx, &run.style.fill, g.bounds, f) {
                self.fold_alpha(ctx, f.ink, &run.style.fill);
                overprint(ctx, run.style.overprint_fill);
                ctx.fill_path(path);
            }
            if run.style.has_stroke() {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                overprint(ctx, run.style.overprint_stroke);
                self.draw_stroke(ctx, f, t.xf, path, FillRule::NonZero, &run.style.stroke_layer(), g.bounds);
            }
        }
        overprint(ctx, false);
        self.draw_inlines(ctx, f, t, g);
        if let Some(all) = &all {
            self.draw_text_items(ctx, f, n, above, all, tb);
        }
    }

    /// Paint some of type `n`'s own fills and strokes (`items`) on its glyph outlines `all`
    /// (document space; `tb` their bounds). Fills composite with their own opacity and blend mode;
    /// strokes take every stroke option, as on paths.
    pub(super) fn draw_text_items(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, items: &[AppearanceItem], all: &BezPath, tb: Rect) {
        for item in items.iter().filter(|i| i.visible() && !i.paint().is_none()) {
            match item {
                AppearanceItem::Fill(fl) => {
                    let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_color::BlendMode::Normal;
                    if layered {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                    }
                    ctx.set_transform(f.view);
                    if paint::set_paint(ctx, &fl.paint, tb, f) {
                        ctx.set_fill_rule(peniko::Fill::NonZero);
                        ctx.fill_path(all);
                    }
                    if layered {
                        ctx.pop_layer();
                    }
                }
                AppearanceItem::Stroke(st) if st.width > 0.0 => {
                    if st.brush.is_none() || !self.draw_brush(ctx, f, n, all, st) {
                        self.draw_stroke(ctx, f, Affine::IDENTITY, all, FillRule::NonZero, st, tb);
                    }
                }
                AppearanceItem::Stroke(_) => {}
            }
        }
    }
}
