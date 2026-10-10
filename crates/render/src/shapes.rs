//! Shape fills, stroke expansion and gradient painting.

use super::{
    Affine, AppearanceItem, Arc, BezPath, BlendMode, Compose, FillRule, Frame, Ink, Mix, Node, Rect, RenderContext, Renderer, Shape, SliceEntry,
    Slices, StrokeAlign, StrokeEntry, StrokeLayer, blend_mode, effects, fill_rule, fx, kurbo, paint, peniko, rects_overlap,
};

impl Renderer {
    pub(super) fn hairline(&mut self, ctx: &mut RenderContext, f: &Frame, bp: &BezPath, rgba: [u8; 4]) {
        ctx.set_transform(Affine::IDENTITY);
        let mut screen = bp.clone();
        screen.apply_affine(f.view);
        ctx.set_stroke(kurbo::Stroke::new(1.0));
        ctx.set_paint(f.ink.fixed(rgba));
        ctx.stroke_path(&screen);
    }

    pub(super) fn draw_shape(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, rule: FillRule) {
        if fx::has_fx(n) {
            return self.draw_shape_fx(ctx, f, n, bp, rule);
        }
        if f.opts.outline {
            self.hairline(ctx, f, bp, [0, 0, 0, 255]);
            return;
        }
        // Bounds only matter to gradients and patterns (their geometry is relative to the object).
        let solid = |p: &vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None);
        let needs_bounds = n.appearance.items.iter().any(|i| match i {
            AppearanceItem::Fill(fl) => !solid(&fl.paint),
            AppearanceItem::Stroke(st) => !solid(&st.paint),
        });
        let bounds = if needs_bounds { bp.bounding_box() } else { Rect::ZERO };
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) => {
                    if !fl.visible || fl.paint.is_none() {
                        continue;
                    }
                    let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_color::BlendMode::Normal;
                    if layered {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                    }
                    ctx.set_transform(f.view);
                    if paint::set_paint(ctx, &fl.paint, bounds, f) {
                        self.fold_alpha(ctx, f.ink, &fl.paint);
                        ctx.set_fill_rule(fill_rule(rule));
                        ctx.fill_path(bp);
                    }
                    if layered {
                        ctx.pop_layer();
                    }
                }
                AppearanceItem::Stroke(st) => {
                    if !st.visible || st.paint.is_none() || st.width <= 0.0 {
                        continue;
                    }
                    if st.brush.is_some() && self.draw_brush(ctx, f, n, bp, st) {
                        continue;
                    }
                    self.draw_stroke(ctx, f, Affine::IDENTITY, bp, rule, st, bounds);
                }
            }
        }
    }

    /// Multiply the folded object opacity into a solid paint.
    pub(super) fn fold_alpha(&self, ctx: &mut RenderContext, ink: Ink, p: &vectorcraft_color::Paint) {
        if self.alpha < 1.0
            && let vectorcraft_color::Paint::Solid { color, .. } = p
        {
            ctx.set_paint(ink.color(color, self.alpha));
        }
    }

    /// Paint stroke `st` of the shape `bp` (in the space `xf` maps to the document; `bounds`: its
    /// geometric bounds there) with its alignment, dashes, profile, arrowheads, opacity and blend.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_stroke(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        xf: Affine,
        bp: &BezPath,
        rule: FillRule,
        st: &StrokeLayer,
        bounds: Rect,
    ) {
        let pieces = effects::stroke::stroke_pieces(bp, st);
        // The line and its heads overlap: they take the opacity once, in one layer, instead of
        // each folding it into its paint.
        let folded = if pieces.heads.is_empty() { 1.0 } else { std::mem::replace(&mut self.alpha, 1.0) };
        let opacity = st.opacity * folded;
        let layered = opacity < 1.0 || st.blend != vectorcraft_color::BlendMode::Normal || st.align == StrokeAlign::Outside;
        if layered {
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, Some(blend_mode(st.blend)), Some(opacity), None, None);
        }
        ctx.set_transform(f.view * xf);
        let closed = effects::stroke::is_closed(bp);
        // Keep hairlines visible when zoomed far out (at least ~1 device pixel).
        let width = effects::stroke::aligned_width(st, closed).max(f.px * 0.5 / xf.determinant().abs().sqrt().max(1e-9));
        let inside = st.align == StrokeAlign::Inside && closed;
        if inside {
            ctx.set_fill_rule(fill_rule(rule));
            ctx.push_clip_layer(bp);
        }
        // One paint box for the line and the heads, so a gradient runs on into the heads.
        let paint_bounds = bounds.inflate(st.width / 2.0, st.width / 2.0);
        if let Some(g) = st.path_gradient() {
            self.fill_path_gradient(ctx, f, bp, rule, st, &pieces, width, &g.gradient);
        } else if paint::set_paint(ctx, &st.paint, paint_bounds, f) {
            self.fold_alpha(ctx, f.ink, &st.paint);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            match self.cached_stroke(f, &pieces.line, st, width) {
                Some(o) => ctx.fill_path(&o),
                None => ctx.fill_path(&effects::stroke::line_outline(&pieces.line, st, width, f.px * 0.25)),
            }
            for head in &pieces.heads {
                ctx.fill_path(&head.outline);
            }
        }
        if inside {
            ctx.pop_layer();
        }
        if st.align == StrokeAlign::Outside && closed {
            // Punch out the interior.
            ctx.set_blend_mode(BlendMode::new(Mix::Normal, Compose::DestOut));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.set_fill_rule(fill_rule(rule));
            ctx.fill_path(bp);
            ctx.set_blend_mode(BlendMode::default());
        }
        if layered {
            ctx.pop_layer();
        }
        if !pieces.heads.is_empty() {
            self.alpha = folded;
        }
    }

    /// Paint the line and arrowheads (`pieces`) of stroke `st` of `bp` (filled with `rule`) with
    /// its gradient `g` laid along or across the path ([`effects::stroke::gradient_slices`]). Each
    /// piece clips the slices, grown a pixel into each other and each replacing what it covers, so
    /// neighbours meet without seams and translucent colours paint once.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill_path_gradient(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        bp: &BezPath,
        rule: FillRule,
        st: &StrokeLayer,
        pieces: &effects::stroke::StrokePieces,
        width: f64,
        g: &vectorcraft_color::Gradient,
    ) {
        let level = (f.px * 0.25).log2().floor() as i32;
        let make = || -> Slices {
            let tol = 2f64.powi(level);
            // Reach a pixel past the stroke, so its antialiased edge has colour under it.
            let slices = effects::stroke::gradient_slices(bp, rule, st, tol, f.px + tol, f.px);
            Arc::new(
                slices
                    .into_iter()
                    .map(|s| {
                        let (b, paint) = (s.shape.bounding_box(), s.paint(g));
                        (s.shape, paint, b)
                    })
                    .collect(),
            )
        };
        let slices = match self.cur.clone() {
            Some(node) => {
                let key = (st as *const StrokeLayer as usize, level);
                match self.stroke_slices.get_mut(&key) {
                    Some(e) if Arc::ptr_eq(&e.node, &node) => {
                        e.stamp = self.stamp;
                        e.slices.clone()
                    }
                    _ => {
                        let slices = make();
                        self.stroke_slices.insert(key, SliceEntry { node, slices: slices.clone(), stamp: self.stamp });
                        slices
                    }
                }
            }
            None => make(),
        };
        let line = match self.cached_stroke(f, &pieces.line, st, width) {
            Some(o) => o,
            None => Arc::new(effects::stroke::line_outline(&pieces.line, st, width, f.px * 0.25)),
        };
        ctx.set_fill_rule(peniko::Fill::NonZero);
        for clip in std::iter::once(&*line).chain(pieces.heads.iter().map(|h| &h.outline)) {
            let cb = clip.bounding_box();
            if clip.elements().is_empty() || !cb.is_finite() {
                continue;
            }
            ctx.push_clip_layer(clip);
            ctx.set_blend_mode(BlendMode::new(Mix::Normal, Compose::Copy));
            for (shape, paint, b) in slices.iter() {
                if rects_overlap(*b, cb) && paint::set_paint(ctx, paint, *b, f) {
                    ctx.fill_path(shape);
                }
            }
            ctx.set_blend_mode(BlendMode::default());
            ctx.pop_layer();
        }
    }

    /// The line part of a stroke of the fast-path node `self.cur` (see
    /// [`effects::stroke::line_outline`]), cached across frames. The tolerance (vello's 0.25
    /// device px) is bucketed to powers of two, never coarser than needed, so zooming reuses
    /// outlines until the level changes.
    pub(super) fn cached_stroke(&mut self, f: &Frame, line: &BezPath, st: &StrokeLayer, width: f64) -> Option<Arc<BezPath>> {
        let node = self.cur.clone()?;
        let level = (f.px * 0.25).log2().floor() as i32;
        let key = (st as *const StrokeLayer as usize, width.to_bits(), level);
        if let Some(e) = self.strokes.get_mut(&key)
            && Arc::ptr_eq(&e.node, &node)
        {
            e.stamp = self.stamp;
            return Some(e.outline.clone());
        }
        let outline = Arc::new(effects::stroke::line_outline(line, st, width, 2f64.powi(level)));
        self.strokes.insert(key, StrokeEntry { node, outline: outline.clone(), stamp: self.stamp });
        Some(outline)
    }
}
