//! Envelope warp styles, deformation surfaces and point mapping.

use super::{
    Affine, Arc, Color, Coons, Deserialize, GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, MeshPoint, Node, NodeId, NodeKind, PI, PathData, Point,
    Rect, Serialize, Vec2, yes,
};

/// How an envelope distorts its content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EnvelopeKind {
    /// One of the 15 warp styles (`arc`, `arcLower`, … — see [`WarpStyle::from_id`]); bend and
    /// distortions in percent (-100..100).
    Warp {
        style: String,
        #[serde(default)]
        bend: f64,
        #[serde(default)]
        h: f64,
        #[serde(default)]
        v: f64,
        #[serde(default = "yes")]
        horizontal: bool,
    },
    /// A `(rows+1)×(cols+1)` grid of points (row-major) the content's bounding box maps onto.
    Mesh {
        rows: u32,
        cols: u32,
        points: Vec<Point>,
        /// Each point's bezier handles (offsets, see [`H_RIGHT`] …), row-major like `points`.
        /// Empty: the smooth (Catmull-Rom) mesh through the points, as before handles existed;
        /// editing the mesh's points or handles on the canvas fills them in.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        handles: Vec<[Vec2; 4]>,
    },
    /// The content's bounding box maps onto the outline of this path.
    TopObject { path: PathData },
}

/// Envelope Options → Preserve Shape Using: how a distorted raster keeps the envelope's shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreserveShape {
    #[default]
    ClippingMask,
    Transparency,
}

impl PreserveShape {
    pub fn id(self) -> &'static str {
        match self {
            Self::ClippingMask => "clippingMask",
            Self::Transparency => "transparency",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "clippingMask" => Some(Self::ClippingMask),
            "transparency" => Some(Self::Transparency),
            _ => None,
        }
    }
}

/// Envelope Options besides Fidelity. `Default` is what envelopes saved before these options
/// existed do (their appearance applies after the distortion); new envelopes start from
/// [`EnvelopeOptions::NEW`], as in the reference app.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvelopeOptions {
    /// Rasters: Anti-Alias.
    pub anti_alias: bool,
    /// Rasters: Preserve Shape Using.
    pub preserve_shape: PreserveShape,
    /// Distort Appearance: strokes and live effects are applied before the distortion, so they
    /// bend with the art (off: they apply to the distorted paths).
    pub distort_appearance: bool,
    /// Distort Linear Gradients (with Distort Appearance): placed gradients bend with the art.
    pub distort_linear_gradients: bool,
    /// Distort Pattern Fills (with Distort Appearance): pattern tiles bend with the art.
    pub distort_pattern_fills: bool,
}

impl Default for EnvelopeOptions {
    fn default() -> Self {
        Self {
            anti_alias: true,
            preserve_shape: PreserveShape::ClippingMask,
            distort_appearance: false,
            distort_linear_gradients: false,
            distort_pattern_fills: false,
        }
    }
}

impl EnvelopeOptions {
    /// The options a new envelope gets (the reference app's defaults).
    pub const NEW: Self = Self {
        anti_alias: true,
        preserve_shape: PreserveShape::ClippingMask,
        distort_appearance: true,
        distort_linear_gradients: false,
        distort_pattern_fills: false,
    };
    /// Do gradients bend with the art?
    pub fn gradients(&self) -> bool {
        self.distort_appearance && self.distort_linear_gradients
    }
    /// Do pattern tiles bend with the art?
    pub fn patterns(&self) -> bool {
        self.distort_appearance && self.distort_pattern_fills
    }
}

// =====================================================================================
// Warp maps (shared with vectorcraft-effects' Warp effects)
// =====================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpStyle {
    Arc,
    ArcLower,
    ArcUpper,
    Arch,
    Bulge,
    ShellLower,
    ShellUpper,
    Flag,
    Wave,
    Fish,
    Rise,
    Fisheye,
    Inflate,
    Squeeze,
    Twist,
}

impl WarpStyle {
    /// From the id suffix (`arc`, `arcLower`, …).
    pub fn from_id(s: &str) -> Option<Self> {
        use WarpStyle::*;
        Some(match s {
            "arc" => Arc,
            "arcLower" => ArcLower,
            "arcUpper" => ArcUpper,
            "arch" => Arch,
            "bulge" => Bulge,
            "shellLower" => ShellLower,
            "shellUpper" => ShellUpper,
            "flag" => Flag,
            "wave" => Wave,
            "fish" => Fish,
            "rise" => Rise,
            "fisheye" => Fisheye,
            "inflate" => Inflate,
            "squeeze" => Squeeze,
            "twist" => Twist,
            _ => return None,
        })
    }
}

/// The warp map on normalised coordinates: `b` = bend (-1..1), `dh`/`dv` = distortion (-1..1).
pub fn warp_point(style: WarpStyle, b: f64, dh: f64, dv: f64, x: f64, y: f64) -> (f64, f64) {
    use WarpStyle::*;
    let t = (y + 1.0) / 2.0; // 0 at top, 1 at bottom
    let par = 1.0 - x * x; // parabola: 1 at centre, 0 at sides
    let (mut x2, mut y2) = match style {
        Arc => {
            if b.abs() < 1e-6 {
                (x, y)
            } else {
                // Bend the centre line into a circular arc of equal length (sweep = |b|·180°).
                let sweep = b.abs() * PI;
                let r0 = 2.0 / sweep;
                let yy = if b > 0.0 { y } else { -y };
                let a = x / r0;
                let r = r0 - yy;
                let (s, c) = a.sin_cos();
                let (nx, ny) = (r * s, r0 - r * c);
                (nx, if b > 0.0 { ny } else { -ny })
            }
        }
        ArcLower => (x, y + b * par * t),
        ArcUpper => (x, y - b * par * (1.0 - t)),
        Arch => (x, y - b * par),
        Bulge => (x, y + b * par * y),
        ShellLower => (x * (1.0 - 0.5 * b * (1.0 - t)), y + b * par * t),
        ShellUpper => (x * (1.0 - 0.5 * b * t), y - b * par * (1.0 - t)),
        Flag => (x, y + 0.5 * b * (PI * x).sin()),
        Wave => (x + 0.1 * b * (PI * y).sin(), y + 0.3 * b * (2.0 * PI * x).sin() * (0.5 + 0.5 * t)),
        Fish => (x, y * (1.0 + 0.5 * b * (0.75 * PI * (x + 1.0)).sin())),
        Rise => (x, y - b * ((x + 1.0) / 2.0).powi(2) * 2.0 + b),
        Fisheye => {
            let r2 = x * x + y * y;
            let s = if r2 < 1.0 { 1.0 + 0.5 * b * (1.0 - r2) } else { 1.0 };
            (x * s, y * s)
        }
        Inflate => (x * (1.0 + 0.5 * b * (1.0 - y * y)), y * (1.0 + 0.5 * b * par)),
        Squeeze => (x * (1.0 - 0.5 * b * (1.0 - y * y)), y * (1.0 + 0.3 * b * par)),
        Twist => {
            let r = (x * x + y * y).sqrt();
            let a = -b * PI * 0.5 * (1.0 - r / std::f64::consts::SQRT_2).max(0.0);
            let (s, c) = a.sin_cos();
            (x * c - y * s, x * s + y * c)
        }
    };
    // Perspective-like distortion: horizontal narrows one side, vertical one end.
    if dh != 0.0 {
        y2 *= (1.0 + dh * x2 * 0.5).max(0.0);
    }
    if dv != 0.0 {
        x2 *= (1.0 + dv * y2 * 0.5).max(0.0);
    }
    (x2, y2)
}

// =====================================================================================
// Envelopes
// =====================================================================================

/// Union of the geometric bounds of `nodes`.
pub fn nodes_bounds(nodes: &[Arc<Node>]) -> Option<Rect> {
    nodes.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric_bounds()))
}

/// A `(rows+1)×(cols+1)` grid over `r` (row-major).
pub fn grid_points(r: Rect, rows: u32, cols: u32) -> Vec<Point> {
    let mut v = Vec::with_capacity(((rows + 1) * (cols + 1)) as usize);
    for i in 0..=rows {
        for j in 0..=cols {
            v.push(Point::new(r.x0 + r.width() * j as f64 / cols.max(1) as f64, r.y0 + r.height() * i as f64 / rows.max(1) as f64));
        }
    }
    v
}

fn catmull(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f64) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Point map from the unit square onto a point grid (tensor-product Catmull-Rom; linear
/// extrapolation at the borders so an undistorted grid is the identity, also past the square).
pub(super) fn grid_eval(rows: usize, cols: usize, pts: &[Point], u: f64, v: f64) -> Point {
    let at = |r: i64, c: i64| -> Vec2 {
        let rr = r.clamp(0, rows as i64);
        let cc = c.clamp(0, cols as i64);
        let base = pts[rr as usize * (cols + 1) + cc as usize].to_vec2();
        // Linear extrapolation outside the grid.
        let er = r - rr;
        let ec = c - cc;
        let mut p = base;
        if er != 0 && rows >= 1 {
            let inner = (rr - er.signum()).clamp(0, rows as i64);
            p += (base - pts[inner as usize * (cols + 1) + cc as usize].to_vec2()) * er.abs() as f64;
        }
        if ec != 0 && cols >= 1 {
            let inner = (cc - ec.signum()).clamp(0, cols as i64);
            p += (base - pts[rr as usize * (cols + 1) + inner as usize].to_vec2()) * ec.abs() as f64;
        }
        p
    };
    // Past the grid's edges the outer patches go on (art can reach past the content's box).
    let (su, sv) = (u * cols as f64, v * rows as f64);
    let (ci, ri) = ((su.floor() as i64).clamp(0, cols as i64 - 1), (sv.floor() as i64).clamp(0, rows as i64 - 1));
    let (fu, fv) = (su - ci as f64, sv - ri as f64);
    let mut rowsv = [Vec2::ZERO; 4];
    for (k, rv) in rowsv.iter_mut().enumerate() {
        let r = ri - 1 + k as i64;
        *rv = catmull(at(r, ci - 1), at(r, ci), at(r, ci + 1), at(r, ci + 2), fu);
    }
    catmull(rowsv[0], rowsv[1], rowsv[2], rowsv[3], fv).to_point()
}

/// The surface (u, v ∈ [0,1]) an envelope of `kind` maps content onto, for content whose bounds in
/// the envelope's own `frame` (its axes → the document) are `src`: a warp bends along the frame's
/// axes; a mesh's points and a top object's path are in the document already.
fn surface_of<'a>(kind: &'a EnvelopeKind, src: Rect, frame: Affine) -> Box<dyn Fn(f64, f64) -> Point + 'a> {
    let flat = move |u: f64, v: f64| frame * Point::new(src.x0 + u * src.width(), src.y0 + v * src.height());
    match kind {
        EnvelopeKind::Warp { style, bend, h: dh, v: dv, horizontal } => {
            let st = WarpStyle::from_id(style).unwrap_or(WarpStyle::Arc);
            let b = bend.clamp(-100.0, 100.0) / 100.0;
            let (dh, dv) = (dh.clamp(-100.0, 100.0) / 100.0, dv.clamp(-100.0, 100.0) / 100.0);
            let c = src.center();
            let (hw, hh) = (src.width() / 2.0, src.height() / 2.0);
            let horizontal = *horizontal;
            Box::new(move |u: f64, v: f64| {
                let (x, y) = (2.0 * u - 1.0, 2.0 * v - 1.0);
                let (x2, y2) = if horizontal {
                    warp_point(st, b, dh, dv, x, y)
                } else {
                    let (a, b2) = warp_point(st, b, dh, dv, y, x);
                    (b2, a)
                };
                frame * Point::new(c.x + x2 * hw, c.y + y2 * hh)
            })
        }
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            if !handles.is_empty()
                && let Some(m) = envelope_grid(*rows, *cols, points, handles)
            {
                return Box::new(move |u: f64, v: f64| mesh_at(&m, u, v));
            }
            if *rows == 0 || *cols == 0 || points.len() as u64 != (u64::from(*rows) + 1) * (u64::from(*cols) + 1) {
                return Box::new(flat);
            }
            let (rows, cols) = (*rows as usize, *cols as usize);
            Box::new(move |u: f64, v: f64| grid_eval(rows, cols, points, u, v))
        }
        EnvelopeKind::TopObject { path } => match Coons::from_path(path) {
            Some(co) => Box::new(move |u: f64, v: f64| co.eval(u, v)),
            None => Box::new(flat),
        },
    }
}

/// The envelope's point map for content whose bounding box is `src` (page axes).
pub fn envelope_mapper<'a>(kind: &'a EnvelopeKind, src: Rect) -> Box<dyn Fn(Point) -> Point + 'a> {
    let s = surface_of(kind, src, Affine::IDENTITY);
    let (w, h) = (src.width().max(1e-9), src.height().max(1e-9));
    Box::new(move |q: Point| s((q.x - src.x0) / w, (q.y - src.y0) / h))
}

/// Catmull-Rom tangents of a `(rows+1)×(cols+1)` point grid (row-major) as mesh handles, a third
/// of the tangent towards each neighbour (past the borders the grid goes on straight): the curves
/// an envelope mesh without handles of its own runs along between its points.
pub fn smooth_handles(rows: u32, cols: u32, points: &[Point]) -> Vec<[Vec2; 4]> {
    let (rows, cols) = (rows as usize, cols as usize);
    let at = |r: usize, c: usize| points.get(r * (cols + 1) + c).copied().unwrap_or_default();
    let tangent = |prev: Option<Point>, p: Point, next: Option<Point>| match (prev, next) {
        (Some(a), Some(b)) => (b - a) / 2.0,
        (None, Some(b)) => b - p,
        (Some(a), None) => p - a,
        (None, None) => Vec2::ZERO,
    };
    let mut out = Vec::with_capacity(points.len());
    for r in 0..=rows {
        for c in 0..=cols {
            let p = at(r, c);
            let du = tangent(c.checked_sub(1).map(|c| at(r, c)), p, (c < cols).then(|| at(r, c + 1))) / 3.0;
            let dv = tangent(r.checked_sub(1).map(|r| at(r, c)), p, (r < rows).then(|| at(r + 1, c))) / 3.0;
            // Handles pointing out of the grid are unused: they stay at the point.
            let keep = |inside: bool, d: Vec2| if inside { d } else { Vec2::ZERO };
            let mut h = [Vec2::ZERO; 4];
            (h[H_RIGHT], h[H_LEFT], h[H_DOWN], h[H_UP]) = (keep(c < cols, du), keep(c > 0, -du), keep(r < rows, dv), keep(r > 0, -dv));
            out.push(h);
        }
    }
    out
}

/// A mesh envelope's grid (`rows`×`cols` patches, `points` and their `handles`; without handles
/// the smooth ones, see [`smooth_handles`]) as a gradient mesh with black points: what the
/// canvas draws and the Mesh tool and Direct Selection edit. `None` for a malformed grid.
pub fn envelope_grid(rows: u32, cols: u32, points: &[Point], handles: &[[Vec2; 4]]) -> Option<GradientMesh> {
    // (Counts come from files: no overflow on 32-bit targets.)
    if rows == 0 || cols == 0 || points.len() as u64 != (u64::from(rows) + 1) * (u64::from(cols) + 1) {
        return None;
    }
    let smooth;
    let handles = if handles.len() == points.len() {
        handles
    } else {
        smooth = smooth_handles(rows, cols, points);
        &smooth
    };
    let points = points.iter().zip(handles).map(|(p, h)| MeshPoint { p: *p, color: Color::BLACK, opacity: 1.0, handles: *h }).collect();
    Some(GradientMesh { rows, cols, points })
}

/// The point of mesh `m` at (u, v) over the whole mesh (each patch an equal share; past the edges
/// the outer patches go on).
fn mesh_at(m: &GradientMesh, u: f64, v: f64) -> Point {
    let (rows, cols) = (m.rows.max(1) as i64, m.cols.max(1) as i64);
    let (su, sv) = (u * cols as f64, v * rows as f64);
    let (c, r) = ((su.floor() as i64).clamp(0, cols - 1), (sv.floor() as i64).clamp(0, rows - 1));
    m.eval(r as usize, c as usize, su - c as f64, sv - r as f64)
}

/// The envelope `id` is, or the innermost one it sits in.
pub fn envelope_of(doc: &crate::Document, id: NodeId) -> Option<&Node> {
    doc.ancestry(id)?.into_iter().rev().find_map(|a| doc.node(a).filter(|n| matches!(n.kind, NodeKind::Envelope { .. })))
}

/// What a selected envelope shows on the canvas: its mesh lines (a warp's surface on a 4×4 grid,
/// a mesh envelope's own grid) or its top object's outline, and for a mesh envelope the grid to
/// edit (its points and handles).
pub fn envelope_overlay(n: &Node) -> Option<(PathData, Option<GradientMesh>)> {
    let NodeKind::Envelope { kind, .. } = &n.kind else { return None };
    match kind {
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            let grid = envelope_grid(*rows, *cols, points, handles)?;
            Some((grid.lines(), Some(grid)))
        }
        EnvelopeKind::Warp { .. } => Some((EnvelopeMap::of(n)?.surface_mesh(4, 4, Color::BLACK).lines(), None)),
        EnvelopeKind::TopObject { path } => Some((path.clone(), None)),
    }
}

/// The frame an envelope keeps after transform `m` (its frame so far composed with the
/// transform): identity when its axes stay square to the page, unflipped, since evaluating on
/// the page axes gives the same result then.
pub fn envelope_frame(m: Affine) -> Affine {
    let [a, b, c, d, _, _] = m.as_coeffs();
    if b.abs() < 1e-12 && c.abs() < 1e-12 && a > 0.0 && d > 0.0 { Affine::IDENTITY } else { m }
}

/// An envelope's point map: content placed in the envelope's own frame maps, by its bounds there,
/// onto the unit square and from there onto the envelope's surface ([`surface_of`]).
pub struct EnvelopeMap<'a> {
    src: Rect,
    frame: Affine,
    to_frame: Affine,
    surface: Box<dyn Fn(f64, f64) -> Point + 'a>,
}

impl<'a> EnvelopeMap<'a> {
    /// The map of an envelope of `kind` around `content` with `frame`. `None` when the content
    /// has no bounds.
    pub fn new(content: &[Arc<Node>], kind: &'a EnvelopeKind, frame: Affine) -> Option<Self> {
        let frame = if frame.is_finite() && frame.determinant().abs() > 1e-12 { frame } else { Affine::IDENTITY };
        let to_frame = frame.inverse();
        let src = if frame == Affine::IDENTITY {
            nodes_bounds(content)?
        } else {
            content.iter().try_fold(None, |acc, c| {
                let mut m = (**c).clone();
                m.transform(to_frame, false);
                Some(vectorcraft_geom::union_opt(acc, m.geometric_bounds()))
            })??
        };
        Some(Self { src, frame, to_frame, surface: surface_of(kind, src, frame) })
    }

    /// The map of envelope node `n` (`None` for other nodes).
    pub fn of(n: &'a Node) -> Option<Self> {
        match &n.kind {
            NodeKind::Envelope { content, kind, frame, .. } => Self::new(content, kind, *frame),
            _ => None,
        }
    }

    /// The content's bounds in the envelope's frame.
    pub fn src(&self) -> Rect {
        self.src
    }

    /// The surface point at (u, v) ∈ [0,1]².
    pub fn at(&self, u: f64, v: f64) -> Point {
        (self.surface)(u, v)
    }

    /// Where the envelope puts document point `p`.
    pub fn map(&self, p: Point) -> Point {
        let q = self.to_frame * p;
        let (w, h) = (self.src.width().max(1e-9), self.src.height().max(1e-9));
        self.at((q.x - self.src.x0) / w, (q.y - self.src.y0) / h)
    }

    /// A flat `(rows+1)×(cols+1)` grid over the content, in the envelope's frame (row-major).
    pub fn grid(&self, rows: u32, cols: u32) -> Vec<Point> {
        grid_points(self.src, rows, cols).into_iter().map(|p| self.frame * p).collect()
    }

    /// The surface sampled on a `rows`×`cols` grid as a mesh whose handles follow it and whose
    /// points are `color`: Release gives it back as the envelope's shape, Reset with Mesh starts
    /// from it.
    pub fn surface_mesh(&self, rows: u32, cols: u32, color: Color) -> GradientMesh {
        GradientMesh::from_surface(rows, cols, &|u, v| self.at(u, v), &|_, _| color)
    }

    /// The surface's bounds (sampled; a mesh's points and a top object's path bound it exactly).
    pub(super) fn bounds(&self) -> Option<Rect> {
        const N: usize = 16;
        let mut r: Option<Rect> = None;
        for j in 0..=N {
            for i in 0..=N {
                let q = self.at(i as f64 / N as f64, j as f64 / N as f64);
                r = Some(r.map_or(Rect::from_points(q, q), |r| r.union_pt(q)));
            }
        }
        r
    }
}
