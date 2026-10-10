//! Save for Web pixel optimization, palette construction and encoding.

use super::{ColorTable, Document, Indexed, PaletteOptions, WebFormat, WebSettings, gif, jpeg, png, quantize, web};

/// One entry of a palette format's colour table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableColor {
    /// As written.
    pub color: [u8; 3],
    /// As the reduction made it (what the colour table's lists name).
    pub source: [u8; 3],
    pub transparent: bool,
    pub locked: bool,
    pub web_shifted: bool,
}

/// An optimised image file.
#[derive(Clone, Debug, Default)]
pub struct Optimized {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The colour table (palette formats), in file order.
    pub colors: Vec<TableColor>,
    /// The colour table's colours that became transparent.
    pub mapped: Vec<[u8; 3]>,
    /// For [`preview`]: the same pixels in a file every decoder reads (a JPEG with optimised
    /// Huffman tables decodes wrongly in the `image` crate's decoder; a baseline one of the same
    /// quality has the same pixels). Empty when `bytes` serves.
    display: Vec<u8>,
}

impl Optimized {
    /// The file the preview decodes ([`decode`]).
    pub fn display(&self) -> &[u8] {
        if self.display.is_empty() { &self.bytes } else { &self.display }
    }
}

/// `#rrggbb`.
pub fn hex([r, g, b]: [u8; 3]) -> String {
    vectorcraft_color::Color::rgb8(r, g, b).to_hex()
}

/// `rgba` (straight, `w`×`h`) flattened on `matte`.
fn flatten(rgba: &[u8], matte: [u8; 3]) -> impl Iterator<Item = [u8; 3]> + '_ {
    rgba.as_chunks::<4>().0.iter().map(move |p| {
        let a = u32::from(p[3]);
        [0, 1, 2].map(|i| ((u32::from(p[i]) * a + u32::from(matte[i]) * (255 - a) + 127) / 255) as u8)
    })
}

/// The File Info text `s.metadata` lets into the files: `(keyword, text)`.
fn metadata(doc: &Document, s: &WebSettings) -> Vec<(&'static str, String)> {
    doc.metadata.png_text(&doc.title).into_iter().filter(|(k, _)| s.metadata.keeps(k)).map(|(k, v)| (k, v.into_owned())).collect()
}

/// Encode straight RGBA pixels (`w`×`h`) as `s` says; `doc` gives the metadata.
pub fn optimize(doc: &Document, rgba: &[u8], w: u32, h: u32, s: &WebSettings) -> std::result::Result<Optimized, String> {
    encode(doc, rgba, w, h, s, false)
}

/// [`optimize`] for the dialog's preview: also what [`Optimized::display`] decodes.
pub fn preview_image(doc: &Document, rgba: &[u8], w: u32, h: u32, s: &WebSettings) -> std::result::Result<Optimized, String> {
    encode(doc, rgba, w, h, s, true)
}

fn encode(doc: &Document, rgba: &[u8], w: u32, h: u32, s: &WebSettings, preview: bool) -> std::result::Result<Optimized, String> {
    if rgba.len() as u64 != u64::from(w) * u64::from(h) * 4 || w == 0 || h == 0 {
        return Err("Save for Web: the image is empty".into());
    }
    let matte = s.matte_rgb()?;
    let meta = metadata(doc, s);
    let comment = || meta.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
    let png_extras = |file: Vec<u8>| {
        let file = if s.convert_to_srgb { png::with_srgb(file) } else { file };
        let text: Vec<(&str, std::borrow::Cow<str>)> = meta.iter().map(|(k, v)| (*k, v.as_str().into())).collect();
        png::with_text(file, &text)
    };
    let png_options = png::PngOptions { ppi: Some(72.0), interlaced: s.interlaced };
    let mut out = Optimized { width: w, height: h, ..Default::default() };
    out.bytes = match s.format {
        WebFormat::Jpg => {
            let rgb: Vec<u8> = flatten(rgba, matte.unwrap_or([255; 3])).flatten().collect();
            let method = match (s.progressive, s.optimized) {
                (true, _) => jpeg::Method::Progressive,
                (false, true) => jpeg::Method::Optimized,
                (false, false) => jpeg::Method::Baseline,
            };
            let o = jpeg::JpegOptions { color_model: jpeg::ColorModel::Rgb, method, scans: 3, embed_icc: s.embed_profile };
            if preview && method == jpeg::Method::Optimized {
                let baseline = jpeg::JpegOptions { method: jpeg::Method::Baseline, embed_icc: false, ..o.clone() };
                out.display = jpeg::encode(&rgb, w, h, s.quality, None, &baseline)?;
            }
            web::jpeg_comment(jpeg::encode(&rgb, w, h, s.quality, Some(72.0), &o)?, &comment())
        }
        WebFormat::Png24 => {
            let file = match (s.transparency, matte.unwrap_or([255; 3])) {
                (true, _) => png::encode(rgba, w, h, &png_options)?,
                (false, m) => {
                    let opaque: Vec<u8> = flatten(rgba, m).flat_map(|[r, g, b]| [r, g, b, 255]).collect();
                    png::encode(&opaque, w, h, &png_options)?
                }
            };
            png_extras(file)
        }
        WebFormat::Gif | WebFormat::Png8 => {
            let PaletteImage { ix, colors, mapped } = palette_image(rgba, w, h, s, matte)?;
            out.colors = colors;
            out.mapped = mapped;
            match s.format {
                WebFormat::Gif => web::gif_comment(gif::encode(&ix, s.interlaced)?, &comment()),
                _ => png_extras(png::encode_indexed(&ix, &png_options)?),
            }
        }
    };
    Ok(out)
}

/// A palette format's image with the colour table's edits.
struct PaletteImage {
    ix: Indexed,
    /// Its colour table.
    colors: Vec<TableColor>,
    /// The colours mapped to transparent.
    mapped: Vec<[u8; 3]>,
}

fn palette_image(rgba: &[u8], w: u32, h: u32, s: &WebSettings, matte: Option<[u8; 3]>) -> std::result::Result<PaletteImage, String> {
    let table = &s.color_table;
    let locked = ColorTable::colors(&table.locked, "locked")?;
    let to_clear = ColorTable::colors(&table.transparent, "transparent")?;
    let shift = ColorTable::colors(&table.web_shift, "webShift")?;
    let o = PaletteOptions {
        colors: s.colors,
        reduction: s.reduction,
        dither: s.dither,
        dither_amount: s.dither_amount,
        transparency: s.transparency,
        matte,
    };
    let mut ix = quantize::quantize_locked(rgba, w, h, &o, &locked);
    // Map to transparent works on the colours the reduction made.
    let before = ix.palette.clone();
    let clear = ix.transparent.map(usize::from);
    let mapped: Vec<[u8; 3]> = before.iter().enumerate().filter(|(i, c)| Some(*i) != clear && to_clear.contains(c)).map(|(_, c)| *c).collect();
    let remap = web::to_transparent(&mut ix, |i| Some(i) != clear && before.get(i).is_some_and(|c| to_clear.contains(c)));
    let mut sources = ix.palette.clone();
    for (old, new) in remap.iter().enumerate() {
        if let (Some(n), Some(c)) = (new, before.get(old))
            && let Some(slot) = sources.get_mut(usize::from(*n))
        {
            *slot = *c;
        }
    }
    // Web snap, then the colours shifted by hand (locked colours stay as they are).
    let clear = ix.transparent.map(usize::from);
    for (i, c) in ix.palette.iter_mut().enumerate() {
        let src = sources.get(i).copied().unwrap_or(*c);
        if Some(i) == clear || locked.contains(&src) {
            continue;
        }
        if shift.contains(&src) {
            *c = quantize::web_safe(*c);
        } else if let Some(snapped) = web::snap(*c, s.web_snap) {
            *c = snapped;
        }
    }
    if s.format == WebFormat::Gif {
        web::lossy(&mut ix, rgba, s.lossy);
    }
    let order = web::sort(&mut ix, table.sort);
    let sources: Vec<[u8; 3]> = order.iter().map(|&i| sources.get(i).copied().unwrap_or_default()).collect();
    let colors = ix
        .palette
        .iter()
        .zip(&sources)
        .enumerate()
        .map(|(i, (c, src))| TableColor {
            color: *c,
            source: *src,
            transparent: ix.transparent == u8::try_from(i).ok(),
            locked: locked.contains(src),
            web_shifted: shift.contains(src),
        })
        .collect();
    Ok(PaletteImage { ix, colors, mapped })
}

/// An optimised image as straight RGBA → `(pixels, width, height)` (the dialog's preview).
pub fn decode(o: &Optimized) -> std::result::Result<(Vec<u8>, u32, u32), String> {
    let img = image::load_from_memory(o.display()).map_err(|e| format!("the optimised image can't be read back: {e}"))?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}
