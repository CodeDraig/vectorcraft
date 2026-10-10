//! Text RGB palette encoding and decoding.

use super::{Color, Paint, Swatch, SwatchGroup, SwatchLibrary, UniqueNames, group_name, one_line};

pub(super) const GPL_HEADER: &str = "GIMP Palette";

const GPL_GROUP: &str = "# Group:";

pub(super) fn write_gpl(lib: &SwatchLibrary) -> String {
    let mut out = format!("{GPL_HEADER}\nName: {}\nColumns: 0\n#\n", one_line(&lib.name));
    let colors = |out: &mut String, list: &[Swatch]| {
        for w in list {
            if let Some(c) = w.paint.color() {
                let [r, g, b, _] = c.to_rgba8(1.0);
                out.push_str(&format!("{r:3} {g:3} {b:3}\t{}\n", one_line(&w.name)));
            }
        }
    };
    colors(&mut out, &lib.swatches);
    for g in &lib.groups {
        out.push_str(&format!("{GPL_GROUP} {}\n", one_line(&g.name)));
        colors(&mut out, &g.swatches);
    }
    out
}

/// `.gpl`: `Name:` names the library, `# Group: name` starts a colour group, other `#` lines and
/// headers (`Columns:`) are skipped, as are lines that aren't `r g b [name]`. Unnamed colors are
/// named by their values and unnamed groups "Color Group"; a name a color or group already has gets
/// a number ([`UniqueNames`]).
pub(super) fn read_gpl(text: &str, fallback: &str) -> SwatchLibrary {
    let mut lib = SwatchLibrary { name: fallback.into(), ..Default::default() };
    let mut names = UniqueNames::default();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if let Some(g) = line.strip_prefix(GPL_GROUP) {
            lib.groups.push(SwatchGroup { name: names.unique(group_name(g)), swatches: vec![] });
            continue;
        }
        if let Some(n) = line.strip_prefix("Name:") {
            lib.name = n.trim().to_string();
            continue;
        }
        // Comments, headers (`Columns: 4`) and anything else that isn't a colour are skipped.
        let words: Vec<&str> = line.split_whitespace().collect();
        let rgb: Vec<u8> = words.iter().take(3).map_while(|v| v.parse().ok()).collect();
        let [r, g, b] = rgb[..] else { continue };
        let rest = words[3..].join(" ");
        let base = if rest.is_empty() { format!("R={r} G={g} B={b}") } else { rest };
        let w = Swatch { name: names.unique(base), paint: Paint::solid(Color::rgb8(r, g, b)), global: false, spot: false };
        match lib.groups.last_mut() {
            Some(grp) => grp.swatches.push(w),
            None => lib.swatches.push(w),
        }
    }
    lib
}
