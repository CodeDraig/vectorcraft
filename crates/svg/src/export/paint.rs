//! SVG paint definitions, stroke plans and shape styles.

use super::{
    Affine, BezPath, FillLayer, FillRule, GradientKind, GradientPaint, LineCap, LineJoin, MASK, Paint, PathData, Point, Props, Rect, StrokeAlign,
    StrokeLayer, StrokePlan, Writer, doc_path, fmt_num, sanitize_id, stroke,
};

impl Writer<'_> {
    /// A fill or stroke value. Gradients resolve unset geometry against `bounds` (the space
    /// [`Self::xf`] maps into user space, as the renderer does).
    pub(super) fn paint(&mut self, p: &Paint, bounds: Option<Rect>) -> String {
        match p {
            Paint::None => "none".into(),
            Paint::Pattern { pattern, xf } => match self.pattern_def(pattern, *xf) {
                Some(id) => format!("url(#{id})"),
                None => "none".into(),
            },
            Paint::Solid { color, .. } => color.to_hex(),
            Paint::Gradient(g) => format!("url(#{})", self.gradient_def(g, bounds.unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0)))),
        }
    }

    /// A `<pattern>` def for pattern `name` placed by `xf`: one period (super-tile) of the tiling,
    /// with every instance that reaches into it.
    pub(super) fn pattern_def(&mut self, name: &str, xf: Affine) -> Option<String> {
        let doc = self.doc;
        let def = doc.pattern(name)?;
        let m = self.xf * xf;
        let key = format!("{name}|{:?}", m.as_coeffs());
        if let Some(id) = self.patterns.get(&key) {
            return Some(id.clone());
        }
        if self.pattern_nest > 4 {
            return None;
        }
        let id = self.fresh_id("pattern");
        self.patterns.insert(key, id.clone());
        let (pw, ph) = def.period();
        let (saved_body, saved_xf, saved_depth) = (std::mem::take(&mut self.body), self.xf, self.depth);
        self.depth = 3;
        self.pattern_nest += 1;
        for o in def.offsets_covering(Rect::new(0.0, 0.0, pw, ph)) {
            self.xf = def.instance_xf(o);
            for a in &def.art {
                self.node(a);
            }
        }
        self.pattern_nest -= 1;
        let content = std::mem::replace(&mut self.body, saved_body);
        self.xf = saved_xf;
        self.depth = saved_depth;
        let head = format!(
            "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{}\" height=\"{}\" patternTransform=\"{}\">",
            self.num(pw),
            self.num(ph),
            self.matrix(m)
        );
        self.def(1, &head);
        self.defs.push_str(&content);
        self.def(1, "</pattern>");
        Some(id)
    }

    /// The id of a `<linearGradient>`/`<radialGradient>` def for `g` resolved against `bounds`.
    /// Identical gradients share one def; a gradient from a swatch takes the swatch's name as its
    /// id. A midpoint other than halfway is written as an extra stop marked `data-vc-midpoint`
    /// (what import turns back into a midpoint).
    pub(super) fn gradient_def(&mut self, g: &GradientPaint, bounds: Rect) -> String {
        if g.gradient.kind == GradientKind::Freeform {
            // Object fills and strokes are images (see `raster_paint`); characters' paints aren't.
            self.warn("freeform gradients on characters are written as linear gradients");
        }
        let mut geom = g.resolve(bounds);
        geom.transform(self.xf, g.gradient.kind);
        let (s, e) = (geom.start, geom.end);
        // An off-centre focal point: `fx`/`fy`, in the gradient's own (unsquashed) space.
        let focal = |w: &Self, f: Point| format!(" fx=\"{}\" fy=\"{}\"", w.num(f.x), w.num(f.y));
        let (tag, prefix, attrs) = match g.gradient.kind {
            GradientKind::Radial => {
                let r = self.num((e - s).hypot());
                let attrs = if (geom.aspect - 1.0).abs() < 1e-9 {
                    let f = geom.focal.map(|f| focal(self, f)).unwrap_or_default();
                    format!(" cx=\"{}\" cy=\"{}\" r=\"{r}\"{f}", self.num(s.x), self.num(s.y))
                } else {
                    let v = e - s;
                    let m = Affine::translate(s.to_vec2()) * Affine::rotate(v.y.atan2(v.x)) * Affine::scale_non_uniform(1.0, geom.aspect);
                    let f = geom.focal.map(|f| focal(self, m.inverse() * f)).unwrap_or_default();
                    format!(" cx=\"0\" cy=\"0\" r=\"{r}\"{f} gradientTransform=\"{}\"", self.matrix(m))
                };
                ("radialGradient", "radial-gradient", attrs)
            }
            GradientKind::Linear | GradientKind::Freeform => (
                "linearGradient",
                "linear-gradient",
                format!(" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"", self.num(s.x), self.num(s.y), self.num(e.x), self.num(e.y)),
            ),
        };
        let stops: Vec<String> = g
            .gradient
            .expanded()
            .map(|(offset, c, o, mid)| {
                let op = if o < 1.0 { format!(" stop-opacity=\"{}\"", fmt_num(o as f64, 3)) } else { String::new() };
                let mid = mid.map(|m| format!(" data-vc-midpoint=\"{}\"", fmt_num(m as f64, 4))).unwrap_or_default();
                format!("<stop offset=\"{}\" stop-color=\"{}\"{op}{mid}/>", fmt_num(offset as f64, 4), c.to_hex())
            })
            .collect();
        let key = format!("{tag}{attrs}{}", stops.concat());
        if let Some(id) = self.gradients.get(&key) {
            return id.clone();
        }
        let id = match &g.swatch {
            Some(name) => self.unique_id(&sanitize_id(name)),
            None => self.fresh_id(prefix),
        };
        self.gradients.insert(key, id.clone());
        self.def(1, &format!("<{tag} id=\"{id}\"{attrs} gradientUnits=\"userSpaceOnUse\">"));
        for stop in &stops {
            self.def(2, stop);
        }
        self.def(1, &format!("</{tag}>"));
        id
    }

    pub(super) fn fill_props(&mut self, f: &FillLayer, rule: FillRule, bounds: Option<Rect>, p: &mut Props) {
        let paint = self.paint(&f.paint, bounds);
        p.push(("fill", paint));
        if f.opacity < 1.0 {
            p.push(("fill-opacity", fmt_num(f.opacity as f64, 3)));
        }
        if rule == FillRule::EvenOdd {
            p.push(("fill-rule", "evenodd".into()));
        }
    }

    pub(super) fn stroke_props(&mut self, s: &StrokeLayer, width: f64, bounds: Option<Rect>, p: &mut Props) {
        let paint = self.paint(&s.paint, bounds);
        p.push(("stroke", paint));
        if s.opacity < 1.0 {
            p.push(("stroke-opacity", fmt_num(s.opacity as f64, 3)));
        }
        if (width - 1.0).abs() > 1e-9 {
            p.push(("stroke-width", self.num(width)));
        }
        match s.cap {
            LineCap::Butt => {}
            LineCap::Round => p.push(("stroke-linecap", "round".into())),
            LineCap::Square => p.push(("stroke-linecap", "square".into())),
        }
        match s.join {
            LineJoin::Miter => {
                if (s.miter_limit - 4.0).abs() > 1e-9 {
                    p.push(("stroke-miterlimit", self.num(s.miter_limit)));
                }
            }
            LineJoin::Round => p.push(("stroke-linejoin", "round".into())),
            LineJoin::Bevel => p.push(("stroke-linejoin", "bevel".into())),
        }
        if let Some(d) = &s.dash
            && d.is_dashed()
        {
            p.push(("stroke-dasharray", d.pattern.iter().map(|v| self.num(*v)).collect::<Vec<_>>().join(" ")));
            if d.offset != 0.0 {
                p.push(("stroke-dashoffset", self.num(d.offset)));
            }
        }
    }

    /// How stroke `st` is written (`bp`: the shape in document space, built on first use from
    /// `paths`): its brush art, or a plain stroke or filled outlines matching the canvas.
    pub(super) fn stroke_plan(&mut self, st: &StrokeLayer, bp: &mut Option<BezPath>, paths: &[&PathData]) -> StrokePlan {
        if stroke::is_plain(st) {
            return StrokePlan::Written(stroke::for_writer(&BezPath::new(), st));
        }
        let bp = bp.get_or_insert_with(|| doc_path(paths));
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        match st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) {
            Some(b) => StrokePlan::Brush(vectorcraft_brush::stroke_pieces(b, bp, st)),
            None => StrokePlan::Written(stroke::for_writer(bp, st)),
        }
    }

    /// The `clip-path` (inside) or `mask` (outside) attribute that keeps an aligned stroke on its
    /// side of the shape `d`; `reach` (document space) is what the stroke covers.
    pub(super) fn side_attr(&mut self, side: Option<StrokeAlign>, d: &str, rule: FillRule, reach: Rect) -> String {
        match side {
            None | Some(StrokeAlign::Center) => String::new(),
            Some(StrokeAlign::Inside) => {
                let cid = self.fresh_id("clip-path");
                let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                self.def(2, &format!("<path d=\"{d}\"{r}/>"));
                self.def(1, "</clipPath>");
                format!(" clip-path=\"url(#{cid})\"")
            }
            Some(StrokeAlign::Outside) if self.tiny() => {
                // No masks: clipped by everything around the shape instead.
                let cid = self.fresh_id("clip-path");
                let b = self.xf.transform_rect_bbox(reach).inflate(1.0, 1.0);
                let (x0, y0, x1, y1) = (self.num(b.x0), self.num(b.y0), self.num(b.x1), self.num(b.y1));
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                self.def(2, &format!("<path d=\"M{x0} {y0} H{x1} V{y1} H{x0} Z {d}\" clip-rule=\"evenodd\"/>"));
                self.def(1, "</clipPath>");
                format!(" clip-path=\"url(#{cid})\"")
            }
            Some(StrokeAlign::Outside) => {
                let mid = self.fresh_id("mask");
                let b = self.xf.transform_rect_bbox(reach).inflate(1.0, 1.0);
                let (x, y, w, h) = (self.num(b.x0), self.num(b.y0), self.num(b.width()), self.num(b.height()));
                let fr = if rule == FillRule::EvenOdd { " fill-rule=\"evenodd\"" } else { "" };
                self.def(1, &format!("<mask id=\"{mid}\" {MASK} x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\">"));
                self.def(2, &format!("<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>"));
                self.def(2, &format!("<path d=\"{d}\" fill=\"#000\"{fr}/>"));
                self.def(1, "</mask>");
                format!(" mask=\"url(#{mid})\"")
            }
        }
    }
}
