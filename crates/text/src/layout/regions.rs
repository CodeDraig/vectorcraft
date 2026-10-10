//! Area-text frame cells and polygon span intersections.

use super::{BezPath, LayoutOptions, PathEl, Point, Rect, Shape};

/// One cell of a flattened area-type frame (the whole frame, or one row/column of it).
pub(super) struct Region {
    /// The cell (frame bounds, or a grid cell of them).
    pub(super) cell: Rect,
    /// Inset applied inside the frame edges.
    pub(super) inset: f64,
    pub(super) polys: Vec<Vec<Point>>,
    /// The frame is its own bounding rectangle (spans need no polygon intersection).
    pub(super) rect: bool,
    /// Text Wrap shapes: (polygons, offset, invert).
    pub(super) wraps: Vec<(Vec<Vec<Point>>, f64, bool)>,
    /// Vertical alignment: how far below the top-aligned position the first line starts.
    pub(super) shift: f64,
    /// Vertical justification: extra space added between consecutive lines.
    pub(super) gap: f64,
}

/// Flattened closed polygons of a path.
fn polygons(path: &BezPath) -> Vec<Vec<Point>> {
    let mut polys: Vec<Vec<Point>> = Vec::new();
    kurbo::flatten(path, 0.1, |el| match el {
        PathEl::MoveTo(p) => polys.push(vec![p]),
        PathEl::LineTo(p) => {
            if let Some(v) = polys.last_mut() {
                v.push(p);
            }
        }
        _ => {}
    });
    polys.retain(|p| p.len() >= 3);
    polys
}

/// Inside intervals (even-odd) of the horizontal line at `y` through `polys`.
fn poly_intervals(polys: &[Vec<Point>], y: f64) -> Vec<(f64, f64)> {
    let mut xs = Vec::new();
    for poly in polys {
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if (a.y <= y) != (b.y <= y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect()
}

/// Union of sorted-or-not intervals.
fn union_intervals(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// `a` minus `b` (both unions of disjoint intervals).
fn subtract_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(mut s, e) in a {
        for &(c, d) in b {
            if d <= s || c >= e {
                continue;
            }
            if c > s {
                out.push((s, c));
            }
            s = s.max(d);
        }
        if e > s {
            out.push((s, e));
        }
    }
    out
}

/// `a` ∩ `b`.
fn intersect_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(s, e) in a {
        for &(c, d) in b {
            let (x, y) = (s.max(c), e.min(d));
            if y > x {
                out.push((x, y));
            }
        }
    }
    out
}

impl Region {
    pub(super) fn cells(path: &BezPath, opts: &LayoutOptions, wrap: &[vectorcraft_doc::WrapShape]) -> Vec<Region> {
        let polys = polygons(path);
        let wraps: Vec<(Vec<Vec<Point>>, f64, bool)> =
            wrap.iter().map(|w| (polygons(&w.path.to_bezpath()), w.wrap.offset.max(0.0), w.wrap.invert)).filter(|w| !w.0.is_empty()).collect();
        let bbox = path.bounding_box();
        let rect = polys.len() == 1 && {
            let area: f64 = polys[0].iter().zip(polys[0].iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>().abs() * 0.5;
            (area - bbox.area()).abs() <= bbox.area() * 1e-6 + 1e-9
        };
        let (rows, cols) = (opts.rows.max(1), opts.columns.max(1));
        let gutter = opts.gutter.max(0.0);
        let cw = ((bbox.width() - gutter * (cols - 1) as f64) / cols as f64).max(0.0);
        let rh = ((bbox.height() - gutter * (rows - 1) as f64) / rows as f64).max(0.0);
        let mut out = Vec::with_capacity(rows * cols);
        // Text flows down each column, then across (Illustrator's default "by columns").
        for c in 0..cols {
            for r in 0..rows {
                let x0 = bbox.x0 + c as f64 * (cw + gutter);
                let y0 = bbox.y0 + r as f64 * (rh + gutter);
                let cell = if rows * cols == 1 { bbox } else { Rect::new(x0, y0, x0 + cw, y0 + rh) };
                out.push(Region { cell, inset: opts.inset.max(0.0), polys: polys.clone(), rect, wraps: wraps.clone(), shift: 0.0, gap: 0.0 });
            }
        }
        out
    }

    pub(super) fn top(&self) -> f64 {
        self.cell.y0 + self.inset
    }
    pub(super) fn bottom(&self) -> f64 {
        self.cell.y1 - self.inset
    }

    /// Inside intervals (even-odd) of the horizontal line at `y`, minus the wrap objects (or
    /// inside them, for Invert Wrap). Offsets grow each wrap shape vertically and horizontally.
    pub(super) fn intervals(&self, y: f64) -> Vec<(f64, f64)> {
        let mut iv = if self.polys.is_empty() || self.rect { vec![(self.cell.x0, self.cell.x1)] } else { poly_intervals(&self.polys, y) };
        for (polys, off, invert) in &self.wraps {
            let ys = if *off > 0.0 { vec![y - off, y - off * 0.5, y, y + off * 0.5, y + off] } else { vec![y] };
            let w = union_intervals(ys.into_iter().flat_map(|y| poly_intervals(polys, y)).map(|(a, b)| (a - off, b + off)).collect());
            iv = if *invert { intersect_intervals(&iv, &w) } else { subtract_intervals(&iv, &w) };
        }
        iv
    }

    /// Widest horizontal span inside the frame (and the cell) over the band `top..bottom`.
    pub(super) fn span(&self, top: f64, bottom: f64) -> Option<(f64, f64)> {
        self.spans(top, bottom).into_iter().max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))
    }

    /// Every horizontal span inside the frame (and the cell) over the band `top..bottom`, left to
    /// right (text wraps on both sides of an object).
    pub(super) fn spans(&self, top: f64, bottom: f64) -> Vec<(f64, f64)> {
        let clip = |(a, b): (f64, f64)| {
            let (a, b) = (a.max(self.cell.x0) + self.inset, b.min(self.cell.x1) - self.inset);
            (b > a).then_some((a, b))
        };
        let plain = self.polys.is_empty() || self.rect;
        if plain && self.wraps.is_empty() {
            return (self.cell.width() > 0.0).then_some((self.cell.x0, self.cell.x1)).and_then(clip).into_iter().collect();
        }
        let fb = if plain {
            self.cell
        } else {
            self.polys.iter().flatten().fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| r.union_pt(*p))
        };
        let clamp = |y: f64| if plain { y } else { y.clamp(fb.y0 + 1e-4, fb.y1 - 1e-4) };
        // Sample the band densely enough for curved frames (circles, blobs).
        let samples = 5;
        let mut rows: Vec<Vec<(f64, f64)>> =
            (0..samples).map(|k| self.intervals(clamp(top + (bottom - top) * k as f64 / (samples - 1) as f64))).collect();
        let mid = rows.swap_remove(samples / 2);
        if !self.wraps.is_empty() {
            // Wrap objects split lines: keep exactly what is free on every sampled row.
            let free = rows.iter().fold(mid, |acc, r| intersect_intervals(&acc, r));
            return free.into_iter().filter_map(clip).collect();
        }
        mid.into_iter()
            .filter_map(|(mut a, mut b)| {
                for o in &rows {
                    let best =
                        o.iter().map(|&(c, d)| (a.max(c), b.min(d))).filter(|(c, d)| d > c).max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))?;
                    a = best.0;
                    b = best.1;
                }
                clip((a, b))
            })
            .collect()
    }
}
