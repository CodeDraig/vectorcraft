//! Measurement units, formatting and typed-distance parsing.

use super::{Deserialize, Serialize};

/// Measurement units (display only; the model is always in points).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Points,
    Picas,
    Inches,
    Millimeters,
    Centimeters,
    Pixels,
    FeetInches,
    Meters,
    Yards,
    Feet,
}

impl Unit {
    pub const ALL: [Unit; 10] = [
        Unit::Points,
        Unit::Picas,
        Unit::Inches,
        Unit::Millimeters,
        Unit::Centimeters,
        Unit::Pixels,
        Unit::FeetInches,
        Unit::Meters,
        Unit::Yards,
        Unit::Feet,
    ];
    /// Points per unit.
    pub fn points(self) -> f64 {
        match self {
            Unit::Points | Unit::Pixels => 1.0,
            Unit::Picas => 12.0,
            Unit::Inches => 72.0,
            Unit::Millimeters => 72.0 / 25.4,
            Unit::Centimeters => 72.0 / 2.54,
            Unit::Meters => 72.0 / 0.0254,
            Unit::Yards => 72.0 * 36.0,
            Unit::Feet | Unit::FeetInches => 72.0 * 12.0,
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Points => "pt",
            Unit::Picas => "p",
            Unit::Inches => "in",
            Unit::Millimeters => "mm",
            Unit::Centimeters => "cm",
            Unit::Pixels => "px",
            Unit::FeetInches | Unit::Feet => "ft",
            Unit::Meters => "m",
            Unit::Yards => "yd",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Unit::Points => "Points",
            Unit::Picas => "Picas",
            Unit::Inches => "Inches",
            Unit::Millimeters => "Millimeters",
            Unit::Centimeters => "Centimeters",
            Unit::Pixels => "Pixels",
            Unit::FeetInches => "Feet & Inches",
            Unit::Meters => "Meters",
            Unit::Yards => "Yards",
            Unit::Feet => "Feet",
        }
    }
    /// The rulers' label step at `zoom` (screen pixels per point), in this unit: the first of
    /// 1, 2, 5 × 10ⁿ that puts labels at least 50 pixels apart. Ticks mark every tenth of it.
    pub fn ruler_step(self, zoom: f64) -> f64 {
        const STEPS: [f64; 19] =
            [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0];
        STEPS.iter().copied().find(|s| s * self.points() * zoom >= 50.0).unwrap_or(10000.0)
    }
    /// `v` (points) on the nearest ruler tick at `zoom` ([`Self::ruler_step`]).
    pub fn snap_to_ruler_tick(self, v: f64, zoom: f64) -> f64 {
        let tick = self.ruler_step(zoom) / 10.0 * self.points();
        (v / tick).round() * tick
    }
    pub fn from_pt(self, v: f64) -> f64 {
        v / self.points()
    }
    pub fn to_pt(self, v: f64) -> f64 {
        v * self.points()
    }
    /// Format a point value in this unit the way fields show it (e.g. `12.5 pt`, `3 in`).
    pub fn format(self, pt: f64) -> String {
        format!("{} {}", self.number(pt), self.suffix())
    }
    /// [`Unit::format`] without the suffix (`12.5`), for narrow fields.
    pub fn number(self, pt: f64) -> String {
        // Three decimals for the small units (`595.276 pt`), four for the large ones (`8.2677 in`,
        // `35.2778 mm`, the `0.0078 in` stroke preset); trailing zeros are trimmed (`1 pt` is `1`).
        let decimals = if matches!(self, Unit::Points | Unit::Pixels | Unit::Picas) { 3 } else { 4 };
        let s = format!("{:.*}", decimals, self.from_pt(pt));
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.into() }
    }
    /// Parse `12`, `12pt`, `1in`, `3 mm`, `2p6` (picas+points), simple `+ - * /` arithmetic.
    ///
    /// A unit after a `*` or `/` operand measures the whole expression (`1080/2 px` is 540 px), so
    /// math typed before a field's unit suffix works.
    pub fn parse(self, s: &str) -> Option<f64> {
        parse_measure(s, self).filter(|v| v.is_finite())
    }
    /// The value naming this unit in the Units preferences (`points`, `millimeters`,
    /// `feetInches`).
    pub fn key(self) -> &'static str {
        match self {
            Unit::Points => "points",
            Unit::Picas => "picas",
            Unit::Inches => "inches",
            Unit::Millimeters => "millimeters",
            Unit::Centimeters => "centimeters",
            Unit::Pixels => "pixels",
            Unit::FeetInches => "feetInches",
            Unit::Meters => "meters",
            Unit::Yards => "yards",
            Unit::Feet => "feet",
        }
    }
    /// The unit `name` names: its label, suffix or [`Unit::key`] in any case, ignoring spaces and
    /// punctuation (`Millimeters`, `mm`, `Feet & Inches`, `feetInches`).
    pub fn named(name: &str) -> Option<Unit> {
        let letters = |s: &'static str| s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        let given = || name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        Unit::ALL.into_iter().find(|u| letters(u.label()).eq(given()) || u.suffix().chars().eq(given()))
    }
    /// A point value as the canvas measurement labels show it: two decimals and the suffix
    /// (`12.50 mm`).
    pub fn readout(self, pt: f64) -> String {
        format!("{:.2} {}", self.from_pt(pt), self.suffix())
    }
}

fn parse_measure(s: &str, default: Unit) -> Option<f64> {
    let s = s.trim();
    // Simple arithmetic on the right (Illustrator fields accept "10+5", "100/2", "3in*2").
    for op in ['+', '-', '*', '/'] {
        // Past the first character, so a leading sign isn't an operator.
        let skip = s.chars().next().map_or(0, char::len_utf8);
        if let Some(i) = s[skip..].rfind(op).map(|i| i + skip) {
            let (l, r) = (&s[..i], &s[i + 1..]);
            if l.trim().is_empty() {
                continue;
            }
            return match op {
                '+' => Some(parse_measure(l, default)? + parse_measure(r, default)?),
                '-' => Some(parse_measure(l, default)? - parse_measure(r, default)?),
                _ => {
                    // A factor or divisor is a plain number; a unit after it is the expression's.
                    let (k, unit) = number_unit(r)?;
                    let a = parse_measure(l, unit.unwrap_or(default))?;
                    if op == '*' {
                        Some(a * k)
                    } else if k == 0.0 {
                        None
                    } else {
                        Some(a / k)
                    }
                }
            };
        }
    }
    if let Some((p, pt)) = s.split_once('p')
        && !p.is_empty()
        && p.trim().parse::<f64>().is_ok()
        && (pt.is_empty() || pt.trim().parse::<f64>().is_ok())
        && !s.ends_with("pt")
        && !s.ends_with("px")
    {
        return Some(p.trim().parse::<f64>().ok()? * 12.0 + pt.trim().parse::<f64>().unwrap_or(0.0));
    }
    let (v, unit) = number_unit(s)?;
    Some(unit.unwrap_or(default).to_pt(v))
}

/// A number with an optional unit suffix (`12`, `3 mm`, `2in`) → (the number, its unit).
fn number_unit(s: &str) -> Option<(f64, Option<Unit>)> {
    let s = s.trim();
    let num_end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(s.len());
    let v: f64 = s.get(..num_end)?.trim().parse().ok()?;
    let unit = match s.get(num_end..)?.trim() {
        "" => None,
        "pt" => Some(Unit::Points),
        "px" => Some(Unit::Pixels),
        "in" | "\"" => Some(Unit::Inches),
        "mm" => Some(Unit::Millimeters),
        "cm" => Some(Unit::Centimeters),
        "m" => Some(Unit::Meters),
        "ft" | "'" => Some(Unit::Feet),
        "yd" => Some(Unit::Yards),
        "p" | "pc" => Some(Unit::Picas),
        _ => return None,
    };
    Some((v, unit))
}

/// A unitless field value (percent, degrees, counts) with the same `+ - * /` arithmetic as
/// [`Unit::parse`] (`45*2`, `100/3`); text other than digits, `.` and operators reads as nothing.
pub fn parse_number(s: &str) -> Option<f64> {
    if !s.chars().all(|c| c.is_ascii_digit() || " .+-*/".contains(c)) {
        return None;
    }
    // A leading `+` is a sign (`+5`), as a plain number read it.
    let s = s.trim();
    Unit::Points.parse(s.strip_prefix('+').unwrap_or(s))
}
