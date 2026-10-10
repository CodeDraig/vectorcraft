//! Binary swatch exchange encoding, parsing and input bounds.

use super::{Color, Paint, Swatch, SwatchGroup, SwatchLibrary, UniqueNames, group_name};

pub(super) const ASE_SIGNATURE: &[u8] = b"ASEF";

pub(super) const ASE_GROUP_START: u16 = 0xC001;

pub(super) const ASE_GROUP_END: u16 = 0xC002;

pub(super) const ASE_COLOR: u16 = 0x0001;

/// The most colors and groups read from one `.ase` file.
pub(super) const ASE_MAX_ENTRIES: usize = 100_000;

/// The most UTF-16 code units of a name that are kept.
const ASE_MAX_NAME: usize = 1024;

const ASE_CUT_SHORT: &str = "the swatch exchange file is cut short";

/// `.ase`, as [`read_ase`] reads it: version 1.0, the ungrouped colors, then each color group as a
/// group start (its name), its colors and a group end, also for a group without colors. A color is
/// written in its own model: RGB, CMYK and gray components from 0 to 1 (gray as a level, 1 is
/// white), Lab lightness as a fraction of 100 and a and b from −128 to 127. Components are clamped
/// to those ranges, and a component that isn't a finite number is written as 0. The color type is
/// 1 for a spot color, 0 for another global color and 2 for a process color.
///
/// A swatch exchange file holds solid colors only: gradients, patterns and None are left out, and
/// a tint swatch is written as the color it shows, a process color without its link to its base.
/// The file has no library name ([`read_ase`] names a library after its file). More than
/// [`ASE_MAX_ENTRIES`] colors and groups is an error.
pub(super) fn write_ase(lib: &SwatchLibrary) -> Result<Vec<u8>, String> {
    let solid = |list: &[Swatch]| list.iter().filter(|w| w.paint.color().is_some()).count();
    let entries = solid(&lib.swatches) + lib.groups.iter().map(|g| 1 + solid(&g.swatches)).sum::<usize>();
    if entries > ASE_MAX_ENTRIES {
        return Err(format!(
            "VectorCraft reads at most {ASE_MAX_ENTRIES} colors and groups from a swatch exchange file, and this library has {entries}"
        ));
    }
    let mut body = vec![];
    let mut blocks = put_ase_colors(&mut body, &lib.swatches)?;
    for g in &lib.groups {
        let mut name = vec![];
        put_ase_name(&mut name, &g.name)?;
        put_ase_block(&mut body, ASE_GROUP_START, &name)?;
        blocks += 2 + put_ase_colors(&mut body, &g.swatches)?;
        put_ase_block(&mut body, ASE_GROUP_END, &[])?;
    }
    let blocks = u32::try_from(blocks).map_err(|_| "too many swatch exchange blocks".to_string())?;
    let mut out = ASE_SIGNATURE.to_vec();
    out.extend(1u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(blocks.to_be_bytes());
    out.extend(body);
    Ok(out)
}

/// The color blocks of the solid colors in `list` → how many were written.
fn put_ase_colors(out: &mut Vec<u8>, list: &[Swatch]) -> Result<usize, String> {
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let ab = |v: f32| if v.is_finite() { v.clamp(-128.0, 127.0) } else { 0.0 };
    let mut n = 0;
    for (w, color) in list.iter().filter_map(|w| Some((w, w.paint.color()?))) {
        let mut body = vec![];
        put_ase_name(&mut body, &w.name)?;
        let (model, values): (&[u8; 4], Vec<f32>) = match color {
            Color::Rgb { r, g, b } => (b"RGB ", vec![unit(r), unit(g), unit(b)]),
            Color::Cmyk { c, m, y, k } => (b"CMYK", vec![unit(c), unit(m), unit(y), unit(k)]),
            Color::Lab { l, a, b } => (b"LAB ", vec![unit(l / 100.0), ab(a), ab(b)]),
            Color::Gray { k } => (b"Gray", vec![1.0 - unit(k)]),
        };
        body.extend(model);
        body.extend(values.iter().flat_map(|v| v.to_be_bytes()));
        let kind: u16 = if w.spot {
            1
        } else if w.global {
            0
        } else {
            2
        };
        body.extend(kind.to_be_bytes());
        put_ase_block(out, ASE_COLOR, &body)?;
        n += 1;
    }
    Ok(n)
}

/// A block: its type, the length of its body and the body.
fn put_ase_block(out: &mut Vec<u8>, kind: u16, body: &[u8]) -> Result<(), String> {
    let len = u32::try_from(body.len()).map_err(|_| "a swatch exchange block is too long".to_string())?;
    out.extend(kind.to_be_bytes());
    out.extend(len.to_be_bytes());
    out.extend_from_slice(body);
    Ok(())
}

/// A name as `.ase` stores it: a `u16` count of UTF-16 code units, the terminating zero included,
/// then the units. NUL characters are left out, and a name longer than [`ASE_MAX_NAME`] code units
/// is cut before the character that would pass that limit.
fn put_ase_name(out: &mut Vec<u8>, name: &str) -> Result<(), String> {
    let mut units: Vec<u16> = vec![];
    for c in name.chars().filter(|&c| c != '\0') {
        let mut buf = [0; 2];
        let encoded = c.encode_utf16(&mut buf);
        if units.len() + encoded.len() > ASE_MAX_NAME {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units.push(0);
    let count = u16::try_from(units.len()).map_err(|_| "a swatch exchange name is too long".to_string())?;
    out.extend(count.to_be_bytes());
    out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    Ok(())
}

/// `.ase`, from public descriptions of the format: the layout from
/// <https://www.selapa.net/swatches/colors/fileformats.php>, the component ranges from
/// <https://github.com/nsfmc/swatch>. After the signature come, big-endian, a `u16` major (1) and
/// minor version, a `u32` block count and the blocks, each a `u16` type, a `u32` body length and
/// the body. A group start's body is a name. A color's body is a name, a four-character model
/// (`RGB `, `CMYK`, `LAB `, `Gray`), its components as `f32` (3, 4, 3 and 1 of them) and a `u16`
/// color type (0 global, 1 spot, 2 process). RGB, CMYK and gray components run from 0 to 1 (a
/// gray level of 1 is white), Lab lightness from 0 to 1 and a and b from −128 to 127. A lightness
/// above 1 is read as L* itself. A name is a `u16` count of UTF-16 code units, the terminating zero
/// included, then the units.
///
/// Color groups become groups: a group start inside an open group ends that group, and a group
/// left open ends with the file. Blocks of other types and colors in other models are skipped.
/// Unnamed colors are named by their values and unnamed groups "Color Group"; a name a color or
/// group already has gets a number ([`UniqueNames`]). A truncated file, a major version other than
/// 1 or more than [`ASE_MAX_ENTRIES`] colors and groups is an error.
pub(super) fn read_ase(bytes: &[u8], fallback: &str) -> Result<SwatchLibrary, String> {
    let mut data = AseData(bytes);
    data.bytes(ASE_SIGNATURE.len())?;
    let (major, minor) = (data.u16()?, data.u16()?);
    if major != 1 {
        return Err(format!("swatch exchange version {major}.{minor} isn't supported (only 1.x)"));
    }
    let blocks = data.u32()?;
    let mut lib = SwatchLibrary { name: fallback.into(), ..Default::default() };
    let mut names = UniqueNames::default();
    let (mut in_group, mut entries) = (false, 0usize);
    for _ in 0..blocks {
        let kind = data.u16()?;
        let len = usize::try_from(data.u32()?).unwrap_or(usize::MAX);
        let mut body = AseData(data.bytes(len)?);
        if matches!(kind, ASE_GROUP_START | ASE_COLOR) {
            entries += 1;
            if entries > ASE_MAX_ENTRIES {
                return Err(format!("the swatch exchange file holds more than {ASE_MAX_ENTRIES} colors and groups"));
            }
        }
        match kind {
            ASE_GROUP_START => {
                lib.groups.push(SwatchGroup { name: names.unique(group_name(&body.name()?)), swatches: vec![] });
                in_group = true;
            }
            ASE_GROUP_END => in_group = false,
            ASE_COLOR => {
                let Some(mut w) = ase_swatch(&mut body)? else { continue };
                w.name = names.unique(std::mem::take(&mut w.name));
                match lib.groups.last_mut().filter(|_| in_group) {
                    Some(g) => g.swatches.push(w),
                    None => lib.swatches.push(w),
                }
            }
            // Other blocks are skipped.
            _ => {}
        }
    }
    Ok(lib)
}

/// The swatch of an `.ase` color block, `None` for a model other than RGB, CMYK, Lab and Gray.
/// Components are clamped to their ranges.
fn ase_swatch(body: &mut AseData) -> Result<Option<Swatch>, String> {
    let name = body.name()?;
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let color = match &body.array::<4>()? {
        b"RGB " => {
            let [r, g, b] = [body.f32()?, body.f32()?, body.f32()?];
            Color::rgb(unit(r), unit(g), unit(b))
        }
        b"CMYK" => {
            let [c, m, y, k] = [body.f32()?, body.f32()?, body.f32()?, body.f32()?];
            Color::cmyk(unit(c), unit(m), unit(y), unit(k))
        }
        // Lightness is stored as a fraction of 100. Some writers store L* itself, which shows as a
        // value above 1.
        b"LAB " => {
            let [l, a, b] = [body.f32()?, body.f32()?, body.f32()?];
            let l = if l > 1.0 { l.min(100.0) } else { unit(l) * 100.0 };
            Color::lab(l, a.clamp(-128.0, 127.0), b.clamp(-128.0, 127.0))
        }
        // A gray level (1 is white); a gray color holds its ink.
        b"Gray" => Color::gray(1.0 - unit(body.f32()?)),
        _ => return Ok(None),
    };
    // Some writers leave the color type out, which makes a process color.
    let kind = if body.0.len() >= 2 { body.u16()? } else { 2 };
    // Spot colors are global.
    let (global, spot) = match kind {
        0 => (true, false),
        1 => (true, true),
        _ => (false, false),
    };
    let name = if name.is_empty() { value_name(color) } else { name };
    Ok(Some(Swatch { name, paint: Paint::solid(color), global, spot }))
}

/// A color's values in its own model, as new swatches are named ("C=10 M=20 Y=30 K=0",
/// "R=255 G=128 B=0", "Gray K=40", "L=52 a=70 b=-30").
fn value_name(c: Color) -> String {
    let pct = |v: f32| (v * 100.0).round();
    let byte = |v: f32| (v * 255.0).round();
    match c {
        Color::Cmyk { c, m, y, k } => format!("C={} M={} Y={} K={}", pct(c), pct(m), pct(y), pct(k)),
        Color::Rgb { r, g, b } => format!("R={} G={} B={}", byte(r), byte(g), byte(b)),
        Color::Gray { k } => format!("Gray K={}", pct(k)),
        // `+ 0.0` turns a rounded -0 into 0.
        Color::Lab { l, a, b } => format!("L={} a={} b={}", l.round() + 0.0, a.round() + 0.0, b.round() + 0.0),
    }
}

/// `.ase` data, read field by field; reading past its end is an error.
struct AseData<'a>(&'a [u8]);

impl<'a> AseData<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(ASE_CUT_SHORT)?;
        self.0 = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(ASE_CUT_SHORT)?;
        self.0 = rest;
        Ok(*head)
    }
    fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_be_bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_be_bytes)
    }
    /// A component; one that isn't a finite number reads as 0.
    fn f32(&mut self) -> Result<f32, String> {
        let v = f32::from_be_bytes(self.array()?);
        Ok(if v.is_finite() { v } else { 0.0 })
    }
    /// A name up to its terminating zero, trimmed. A block that ends before it has no name.
    fn name(&mut self) -> Result<String, String> {
        if self.0.is_empty() {
            return Ok(String::new());
        }
        let n = usize::from(self.u16()?);
        let (units, _) = self.bytes(n * 2)?.as_chunks::<2>();
        let units: Vec<u16> = units.iter().take(ASE_MAX_NAME).map(|u| u16::from_be_bytes(*u)).take_while(|&u| u != 0).collect();
        Ok(String::from_utf16_lossy(&units).trim().to_string())
    }
}
