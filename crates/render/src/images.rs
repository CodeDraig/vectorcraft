//! Image decoding, adjustment caching and pixmap painting.

use super::{
    Affine, Arc, BezPath, Document, FillRule, Frame, HashMap, Pixmap, Rect, RenderContext, Renderer, Shape, fill_rule, paint, peniko, vello_cpu,
};

impl Renderer {
    pub(super) fn draw_image(&mut self, ctx: &mut RenderContext, f: &Frame, im: &vectorcraft_doc::ImageObject) {
        self.draw_image_in(ctx, f, im, None);
    }

    /// Draw an image, or with `area` (a region in the document inside the image's frame, see
    /// [`plain_image`]) the image painted into that region.
    pub(super) fn draw_image_in(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        im: &vectorcraft_doc::ImageObject,
        area: Option<&(BezPath, FillRule)>,
    ) {
        let rect = Rect::new(0.0, 0.0, im.width as f64, im.height as f64);
        // Outline mode draws the image's frame, and with Document Setup → Show Images in Outline
        // Mode its pixels in greyscale under the frame.
        let outline = f.opts.outline;
        // A linked image's preview is cached apart from the file's pixels, which share its key.
        let cache_key = match f.doc.images.get(&im.key) {
            Some(b) if b.is_proxy() => std::borrow::Cow::Owned(format!("{}\u{0}proxy", im.key)),
            _ => std::borrow::Cow::Borrowed(im.key.as_str()),
        };
        let pixels = if outline && !f.doc.setup.outline_images { None } else { self.image_pixmap(f.doc, &im.key, &cache_key, outline) };
        if let Some(pm) = pixels {
            let pm = if outline { pm } else { self.ink_image(&cache_key, &pm, f.ink, f.doc.images.get(&im.key)) };
            let sx = im.width as f64 / pm.width().max(1) as f64;
            let sy = im.height as f64 / pm.height().max(1) as f64;
            let quality = if f.opts.smooth_images { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
            let sampler = peniko::ImageSampler { quality, ..Default::default() };
            ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
            match area {
                Some((bp, rule)) => {
                    ctx.set_transform(f.view);
                    ctx.set_paint_transform(im.xf * Affine::scale_non_uniform(sx, sy));
                    ctx.set_fill_rule(fill_rule(*rule));
                    ctx.fill_path(bp);
                }
                None => {
                    ctx.set_transform(f.view * im.xf);
                    ctx.set_paint_transform(Affine::scale_non_uniform(sx, sy));
                    ctx.fill_rect(&rect);
                }
            }
            ctx.reset_paint_transform();
            // A dimmed layer's images: white over them, so they show at that opacity on paper.
            if let Some(dim) = self.dim_images.filter(|d| *d < 1.0 && !outline) {
                ctx.set_paint(f.ink.fixed([255, 255, 255, ((1.0 - dim) * 255.0).round() as u8]));
                match area {
                    Some((bp, rule)) => {
                        ctx.set_transform(f.view);
                        ctx.set_fill_rule(fill_rule(*rule));
                        ctx.fill_path(bp);
                    }
                    None => {
                        ctx.set_transform(f.view * im.xf);
                        ctx.fill_rect(&rect);
                    }
                }
            }
        }
        if outline {
            let mut p = rect.to_path(0.1);
            p.apply_affine(im.xf);
            self.hairline(ctx, f, &p, [0, 0, 0, 255]);
        }
    }

    /// Cache the pixels of image blob `key` recoloured by `map` as `derived` (the key the adjusted
    /// art's image takes). The latest ones are kept up to a memory budget: dropping one drops the
    /// cached art too, so it is made again.
    pub(super) fn adjusted_image(&mut self, doc: &Document, key: &str, derived: String, map: &vectorcraft_effects::ColorMap) {
        const BUDGET: usize = 256 << 20;
        if self.images.contains_key(&derived) {
            return;
        }
        let Some(src) = self.image_pixmap(doc, key, key, false) else { return };
        let mut pm = (*src).clone();
        let mut memo: HashMap<[u8; 4], [u8; 3]> = HashMap::new();
        for px in pm.data_mut().iter_mut().filter(|p| p.a > 0) {
            let a = px.a as u32;
            let [r, g, b] = *memo.entry([px.r, px.g, px.b, px.a]).or_insert_with(|| {
                // Premultiplied to straight, adjusted, then premultiplied again.
                let straight = [px.r, px.g, px.b].map(|v| ((v as u32 * 255 + a / 2) / a).min(255) as u8);
                map.apply_rgb8(straight).map(|v| ((v as u32 * a + 127) / 255) as u8)
            });
            (px.r, px.g, px.b) = (r, g, b);
        }
        let bytes = pm.data().len() * 4;
        self.images.insert(derived.clone(), Arc::new(pm));
        self.adjusted.push_back((derived, bytes));
        let mut total: usize = self.adjusted.iter().map(|(_, b)| b).sum();
        while total > BUDGET && self.adjusted.len() > 1 {
            let Some((old, b)) = self.adjusted.pop_front() else { break };
            self.images.remove(&old);
            self.fx_arts.clear();
            total -= b;
        }
    }

    /// The decoded pixels of image blob `key` (cached as `cache_key`), or a greyscale copy of them.
    pub(super) fn image_pixmap(&mut self, doc: &Document, key: &str, cache_key: &str, grey: bool) -> Option<Arc<Pixmap>> {
        let colour = match self.images.get(cache_key) {
            Some(p) => p.clone(),
            None => {
                let blob = doc.images.get(key)?;
                match self.cmyk_image(blob, cache_key) {
                    Some(pm) => pm,
                    None => {
                        let pm = Arc::new(paint::decode_pixmap(&blob.bytes)?);
                        self.images.insert(cache_key.to_string(), pm.clone());
                        pm
                    }
                }
            }
        };
        if !grey {
            return Some(colour);
        }
        let grey_key = format!("{cache_key}\u{0}grey");
        if let Some(p) = self.images.get(&grey_key) {
            return Some(p.clone());
        }
        let mut pm = (*colour).clone();
        for px in pm.data_mut() {
            // The luma of premultiplied components is the premultiplied luma.
            let l = (0.2126 * px.r as f32 + 0.7152 * px.g as f32 + 0.0722 * px.b as f32 + 0.5) as u8;
            (px.r, px.g, px.b) = (l, l, l);
        }
        let pm = Arc::new(pm);
        self.images.insert(grey_key, pm.clone());
        Some(pm)
    }
}
