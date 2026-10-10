//! SVG reusable image/symbol references and effect filters.

use super::{
    Affine, BlendMode, HashMap, ImageBlob, ImageMode, LinkedImage, Node, NodeKind, Reuse, SymbolDef, Writer, base64_encode, blend_css, fmt_num,
    image_ext, sanitize_id, web_image, xml_escape,
};

impl Writer<'_> {
    /// The `href` of image `im`: its file with [`ImageMode::Link`] (or when its pixels are
    /// missing), else its bytes. `None` when it has neither.
    pub(super) fn image_href(&mut self, im: &vectorcraft_doc::ImageObject) -> Option<String> {
        let (doc, link) = (self.doc, self.opts.images == ImageMode::Link);
        match (im.link.as_ref().filter(|_| link), doc.images.get(&im.key)) {
            (Some(l), _) => Some(l.path.clone()),
            (None, Some(b)) if !b.bytes.is_empty() => Some(self.blob_href(&web_image(b))),
            _ => im.link.as_ref().map(|l| l.path.clone()),
        }
    }

    /// Images drawn more than once in `art` (the pieces of a distorted image) go into the defs
    /// once; each piece then draws them with `<use>`.
    pub(super) fn share_images(&mut self, art: &Node) {
        let mut seen: HashMap<(String, u32, u32), usize> = HashMap::new();
        art.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind {
                *seen.entry((im.key.clone(), im.width, im.height)).or_default() += 1;
            }
        });
        let mut repeated: Vec<_> = seen.into_iter().filter(|(k, n)| *n > 1 && !self.shared_images.contains_key(k)).map(|(k, _)| k).collect();
        repeated.sort();
        for (key, width, height) in repeated {
            let link = self.linked_of(art, &key);
            let im = vectorcraft_doc::ImageObject { key: key.clone(), width, height, xf: Affine::IDENTITY, link, placement: Default::default() };
            let Some(href) = self.image_href(&im) else { continue };
            let id = self.fresh_id("image");
            self.def(
                0,
                &format!(
                    "<image id=\"{id}\" width=\"{width}\" height=\"{height}\" preserveAspectRatio=\"none\" xlink:href=\"{}\"/>",
                    xml_escape(&href)
                ),
            );
            self.shared_images.insert((key, width, height), id);
        }
    }

    /// The link of the first image with blob `key` in `art`.
    pub(super) fn linked_of(&self, art: &Node, key: &str) -> Option<vectorcraft_doc::LinkInfo> {
        let mut link = None;
        art.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind
                && im.key == key
                && link.is_none()
            {
                link = im.link.clone();
            }
        });
        link
    }

    /// The `href` of image bytes `b`: a `data:` URI, or with [`ImageMode::Link`] a file written
    /// next to the SVG, named after the bytes (image keys such as "raster-1" repeat across
    /// documents).
    pub(super) fn blob_href(&mut self, b: &ImageBlob) -> String {
        if self.opts.images != ImageMode::Link {
            return format!("data:{};base64,{}", b.mime, base64_encode(&b.bytes));
        }
        let name = format!("{}.{}", b.content_key(), image_ext(&b.mime));
        if !self.linked.iter().any(|l| l.name == name) {
            self.linked.push(LinkedImage { name: name.clone(), bytes: b.bytes.clone() });
        }
        name
    }

    /// The id of the `<symbol>` def instance `inst` (transform `xf`) can `<use>`, written on first
    /// use. `None` when the instance needs its own copy of the art: stained by its fill, or moved
    /// by a transform under which the def would not look as the canvas paints it ([`Reuse`]).
    pub(super) fn symbol_use(&mut self, sym: &vectorcraft_doc::Symbol, inst: &Node, xf: Affine) -> Option<String> {
        // SVG Tiny has no `<symbol>`.
        if self.tiny() || vectorcraft_brush::stain(inst).is_some() {
            return None;
        }
        if !self.symbols.contains_key(&sym.name) {
            let reuse = Reuse::of(self.doc, &sym.art);
            self.symbols.insert(sym.name.clone(), SymbolDef { id: None, reuse });
        }
        let def = self.symbols.get(&sym.name)?;
        if !def.reuse.allows(xf) {
            return None;
        }
        if let Some(id) = &def.id {
            return Some(id.clone());
        }
        // The art in symbol space, as its own picture: no knockout around it, and no object ids
        // (every instance shows it).
        let id = self.unique_id(&sanitize_id(&sym.name));
        let saved = (self.xf, self.instance_xf, std::mem::take(&mut self.knockout), std::mem::take(&mut self.names), self.anonymous);
        (self.xf, self.instance_xf, self.anonymous) = (Affine::IDENTITY, Affine::IDENTITY, false);
        self.symbol_nest += 1;
        let art = self.detached(3, |w| w.node(&sym.art));
        self.symbol_nest -= 1;
        (self.xf, self.instance_xf, self.knockout, self.names, self.anonymous) = saved;
        // The symbol's name, when the id had to differ from it, comes back on import.
        let name = if id == sym.name { String::new() } else { format!(" data-name=\"{}\"", xml_escape(&sym.name)) };
        self.def(1, &format!("<symbol id=\"{id}\"{name} overflow=\"visible\">"));
        self.defs.push_str(&art);
        self.def(1, "</symbol>");
        if let Some(def) = self.symbols.get_mut(&sym.name) {
            def.id = Some(id.clone());
        }
        Some(id)
    }

    /// Raster effects (drop shadow, glows, Gaussian blur, feather) as SVG filters: one `<g filter>`
    /// per effect, the first effect innermost (the reference app's stacking order). Photoshop-style
    /// filters have no SVG equivalent: they are left out with a warning (exports from the app turn
    /// such objects into images first).
    pub(super) fn filtered(&mut self, n: &Node) {
        let mut inner = n.clone();
        inner.appearance.effects.retain(|e| !vectorcraft_effects::is_raster(&e.id));
        let opened = self.open_filters(n, &n.appearance.effects);
        self.node_body(&inner);
        self.close_filters(opened);
    }

    /// Open one `<g filter>` per visible raster effect of `effects` (an object's, or one fill or
    /// stroke's) on `n`, outermost last effect; returns how many to close.
    pub(super) fn open_filters(&mut self, n: &Node, effects: &[vectorcraft_doc::Effect]) -> usize {
        use vectorcraft_effects::RasterFx;
        let fx = vectorcraft_effects::raster_effects(effects);
        if self.tiny() {
            if !fx.is_empty() {
                self.warn("SVG Tiny 1.2 has no filters: shadows, glows, blurs and feathers are left out");
            }
            return 0;
        }
        let bounds = n.visual_bounds();
        let reach: f64 = fx.iter().map(|f| f.outset(bounds.unwrap_or_default())).sum::<f64>() + 2.0;
        let region = bounds.map(|b| self.xf.transform_rect_bbox(b).inflate(reach, reach));
        let mut opened = 0;
        for f in fx.iter().rev() {
            if matches!(f, RasterFx::Pixel(_)) {
                self.warn("SVG has no pixel effects such as Radial Blur, Smart Blur or Unsharp Mask: they are left out");
                continue;
            }
            let fid = self.fresh_id("filter");
            let region_attr = match region {
                Some(r) => format!(
                    " filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    self.num(r.x0),
                    self.num(r.y0),
                    self.num(r.width()),
                    self.num(r.height())
                ),
                None => String::new(),
            };
            let sd = |blur: f64| fmt_num((blur / 2.0).max(0.0), 3);
            let flood = |c: &vectorcraft_color::Color, o: f32| {
                format!("<feFlood flood-color=\"{}\" flood-opacity=\"{}\"/>", c.to_hex(), fmt_num(o as f64, 3))
            };
            let body = match f {
                RasterFx::DropShadow { opacity, dx, dy, blur, color, .. } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\"/><feOffset dx=\"{}\" dy=\"{}\" result=\"shadow\"/>{}<feComposite in2=\"shadow\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"paint\"/><feMergeNode in=\"SourceGraphic\"/></feMerge>",
                    sd(*blur),
                    self.num(*dx),
                    self.num(*dy),
                    flood(color, *opacity)
                ),
                RasterFx::OuterGlow { opacity, blur, color, .. } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\" result=\"glow\"/>{}<feComposite in2=\"glow\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"paint\"/><feMergeNode in=\"SourceGraphic\"/></feMerge>",
                    sd(*blur),
                    flood(color, *opacity)
                ),
                RasterFx::InnerGlow { opacity, blur, color, center, .. } => {
                    // Edge: the blurred inverse silhouette; Center: the blurred silhouette; both clipped to the shape.
                    let src = if *center {
                        "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"SD\" result=\"glow\"/>".replace("SD", &sd(*blur))
                    } else {
                        format!(
                            "<feComponentTransfer in=\"SourceAlpha\"><feFuncA type=\"table\" tableValues=\"1 0\"/></feComponentTransfer><feGaussianBlur stdDeviation=\"{}\" result=\"glow\"/>",
                            sd(*blur)
                        )
                    };
                    format!(
                        "{src}{}<feComposite in2=\"glow\" operator=\"in\"/><feComposite in2=\"SourceAlpha\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"SourceGraphic\"/><feMergeNode in=\"paint\"/></feMerge>",
                        flood(color, *opacity)
                    )
                }
                RasterFx::Feather { radius } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\" result=\"soft\"/><feComposite in=\"SourceGraphic\" in2=\"soft\" operator=\"in\" result=\"f\"/><feComposite in=\"f\" in2=\"SourceAlpha\" operator=\"in\"/>",
                    sd(*radius)
                ),
                RasterFx::GaussianBlur { radius } => format!("<feGaussianBlur in=\"SourceGraphic\" stdDeviation=\"{}\"/>", sd(*radius)),
                RasterFx::Pixel(_) => continue,
            };
            // Filters composite normally: a shadow's or glow's other blend mode is recorded for import.
            let mode = match f {
                RasterFx::DropShadow { mode, .. } | RasterFx::OuterGlow { mode, .. } | RasterFx::InnerGlow { mode, .. }
                    if *mode != BlendMode::Normal =>
                {
                    format!(" {}=\"{}\"", crate::import::BLEND, blend_css(*mode))
                }
                _ => String::new(),
            };
            self.def(1, &format!("<filter id=\"{fid}\"{region_attr}{mode} color-interpolation-filters=\"sRGB\">{body}</filter>"));
            self.line(&format!("<g filter=\"url(#{fid})\">"));
            self.depth += 1;
            opened += 1;
        }
        opened
    }

    pub(super) fn close_filters(&mut self, opened: usize) {
        for _ in 0..opened {
            self.depth -= 1;
            self.line("</g>");
        }
    }
}
