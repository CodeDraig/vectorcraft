//! PDF image decoding, caching and image emission.

use super::{Exporter, Image, Size, Surface, embed, xf};

impl Exporter<'_> {
    /// Image `key` placed `size` points wide and high, resampled and compressed as the Compression
    /// settings say ([`crate::images::recode`], [`crate::images::recode_cmyk`] for CMYK images);
    /// cached by key (and size when images are resampled).
    pub(super) fn load_image(&mut self, key: &str, size: (f64, f64)) -> Option<Image> {
        let c = self.compression;
        let sized = [c.color.downsample, c.gray.downsample, c.mono.downsample].iter().any(|d| *d != crate::Downsample::None);
        let cache = if sized { format!("{key}\u{0}{:.3}x{:.3}", size.0, size.1) } else { key.to_string() };
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let interpolate = self.interpolate;
        let doc = self.doc;
        let out = self.out.clone();
        let img = doc.images.get(key).and_then(|blob| {
            let r = if blob.is_cmyk() {
                crate::images::recode_cmyk(blob, size, c, interpolate, out.cmyk_pixels())
            } else {
                crate::images::recode(&blob.bytes, size, c, interpolate, out.pixels())
            };
            if let Some(w) = r.warning {
                self.warn(w);
            }
            r.image.or_else(|| embed(blob, interpolate))
        });
        if img.is_none() {
            self.warn(format!("image '{key}' could not be decoded and was skipped"));
        }
        self.images.insert(cache, img.clone());
        img
    }

    pub(super) fn image(&mut self, s: &mut Surface, im: &vectorcraft_doc::ImageObject) {
        // The size it is placed at: its pixel grid's sides through its transform.
        let [a, b, c, d, ..] = im.xf.as_coeffs();
        let size = (im.width as f64 * a.hypot(b), im.height as f64 * c.hypot(d));
        let Some(img) = self.load_image(&im.key, size) else { return };
        let Some(size) = Size::from_wh(im.width.max(1) as f32, im.height.max(1) as f32) else { return };
        s.push_transform(&xf(im.xf));
        s.draw_image(img, size);
        s.pop();
    }
}
