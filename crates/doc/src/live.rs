//! Live blends, envelope distortions and gradient meshes: their data and pure evaluation.
//!
//! These are *live* objects. The document stores the source objects plus parameters; the
//! renderer and the exporters evaluate them on demand ([`expand_live`], [`expand_deep`]).
//! Everything here is pure geometry so the SVG/PDF exporters (which only depend on this crate)
//! can export the expanded form.
//!
//! - **Blend**: key objects are matched anchor-by-anchor (subpaths resampled to equal anchor
//!   counts), paints/strokes/opacity interpolated; optional spine (keys spread along it by arc
//!   length, optionally rotated to its tangent).
//! - **Envelope**: all descendant geometry of the content is pushed through a point map from the
//!   content's bounding box to a warp style, a Catmull-Rom point mesh or a top object (Coons
//!   patch built from the top path's outline split at its corners).
//! - **Gradient mesh**: a grid of points with colours and tangent handles; each patch is a Coons
//!   patch coloured by bilinear interpolation of its four corners.

mod mesh;
pub use mesh::{GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, MeshAppearance, MeshPoint, MeshQuad, default_fidelity};
mod envelope;
#[cfg(test)]
use envelope::grid_eval;
pub use envelope::{
    EnvelopeKind, EnvelopeMap, EnvelopeOptions, PreserveShape, WarpStyle, envelope_frame, envelope_grid, envelope_mapper, envelope_of,
    envelope_overlay, grid_points, nodes_bounds, smooth_handles, warp_point,
};

use std::f64::consts::PI;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{Affine, Anchor, BezPath, CubicBez, ParamCurve, PathData, Point, Rect, Shape, SubPath, Vec2};

use crate::appearance::{Appearance, AppearanceItem, FillLayer};
use crate::node::{ImageObject, Node, NodeId, NodeKind};

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn fifty() -> f64 {
    50.0
}

// =====================================================================================
// Data
// =====================================================================================

/// Blend Options → Spacing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendSpacing {
    /// Step count chosen from the colour difference (or the distance when colours match).
    #[default]
    SmoothColor,
    /// Specified number of steps between each pair of key objects (1–1000).
    Steps(u32),
    /// Specified distance (pt) between the steps.
    Distance(f64),
}

/// Blend Options → Orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendOrientation {
    #[default]
    AlignToPage,
    AlignToPath,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendSpec {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
    /// Replaced spine. `None` = the straight lines between the key objects' centres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spine: Option<PathData>,
    /// The anchor of the spine's first subpath each key object sits on (a spine edited on the
    /// canvas: moving a key moves its anchor and the other way round). Empty: the keys spread
    /// evenly along the spine by arc length (Replace Spine).
    #[serde(default, rename = "keyAnchors", skip_serializing_if = "Vec::is_empty")]
    pub key_anchors: Vec<u32>,
    /// Per key object, the anchor of its first subpath the blend starts from (Blend tool clicks
    /// on anchor points); `None`: chosen so closed shapes don't twist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starts: Vec<Option<u32>>,
}

impl BlendSpec {
    /// The start anchor of key `i` (see [`Self::starts`]).
    pub fn start(&self, i: usize) -> Option<usize> {
        self.starts.get(i).copied().flatten().map(|a| a as usize)
    }
}

/// Blend Options set with no blend selected: what new blends start with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendDefaults {
    #[serde(default)]
    pub spacing: BlendSpacing,
    #[serde(default)]
    pub orientation: BlendOrientation,
}

// =====================================================================================
// Blends (evaluation in `crate::blend`)
// =====================================================================================

pub use crate::blend::{
    Spine, blend_expand, blend_spine, blend_step_count, lerp_appearance, lerp_color, lerp_node, lerp_paint, lerp_path, pin_spine,
    steps_knockout_shows,
};

/// Segment as a cubic; straight lines get handles at 1/3 and 2/3 so a non-linear map bends them.
fn seg_cubic(sp: &SubPath, i: usize) -> CubicBez {
    let c = sp.segment(i);
    if sp.segment_is_line(i) { CubicBez::new(c.p0, c.p0.lerp(c.p3, 1.0 / 3.0), c.p0.lerp(c.p3, 2.0 / 3.0), c.p3) } else { c }
}

fn poly_len(c: &CubicBez) -> f64 {
    c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3)
}

/// Running length along a polyline: `0, |p0p1|, |p0p1| + |p1p2|, …` (one entry per point).
fn cumulative_lengths(pts: &[Point]) -> Vec<f64> {
    let mut total = 0.0;
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0);
    for w in pts.windows(2) {
        total += w[0].distance(w[1]);
        cum.push(total);
    }
    cum
}

/// Map every point of `path` through the non-linear function `f`. Segments are split into pieces
/// no longer than about `max_piece` (up to 64 per segment) and their control points mapped, which
/// approximates the image curve to O(h²). Closedness and subpath structure are preserved.
pub fn map_nonlinear(path: &PathData, max_piece: f64, f: impl Fn(Point) -> Point) -> PathData {
    let max_piece = max_piece.max(1e-3);
    let mut subs = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let n = sp.anchors.len();
        if n == 0 {
            continue;
        }
        let segs = sp.segment_count();
        let mut res: Vec<Anchor> = Vec::new();
        let mut pending_in: Option<Point> = None;
        for i in 0..n {
            let a = &sp.anchors[i];
            let h_in = pending_in.take().unwrap_or_else(|| f(a.h_in));
            res.push(Anchor { p: f(a.p), h_in, h_out: f(a.h_out), kind: a.kind });
            if i < segs {
                let c = seg_cubic(sp, i);
                let k = ((poly_len(&c) / max_piece).ceil() as usize).clamp(1, 64);
                for j in 0..k {
                    let sub = c.subsegment((j as f64 / k as f64)..((j + 1) as f64 / k as f64));
                    if let Some(last) = res.last_mut() {
                        last.h_out = f(sub.p1);
                    }
                    if j + 1 < k {
                        res.push(Anchor { p: f(sub.p3), h_in: f(sub.p2), h_out: f(sub.p3), kind: Default::default() });
                    } else {
                        pending_in = Some(f(sub.p2));
                    }
                }
            }
        }
        if let Some(h) = pending_in
            && sp.closed
            && let Some(first) = res.first_mut()
        {
            first.h_in = h;
        }
        let anchors = res.into_iter().map(|a| Anchor::with_handles(a.p, a.h_in, a.h_out)).collect();
        subs.push(SubPath::new(anchors, sp.closed));
    }
    PathData::new(subs)
}

// =====================================================================================
// Coons patches from outlines
// =====================================================================================

fn flatten_subpath(sp: &SubPath, tol: f64) -> Vec<Point> {
    let mut bp = BezPath::new();
    sp.to_bezpath_into(&mut bp);
    let mut pts: Vec<Point> = Vec::new();
    vectorcraft_geom::kurbo::flatten(bp.iter(), tol, |el| match el {
        vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) if pts.last().is_none_or(|q| q.distance(p) > 1e-9) => pts.push(p),
        _ => {}
    });
    pts
}

fn resample(poly: &[Point], m: usize) -> Vec<Point> {
    if poly.is_empty() {
        return vec![Point::ZERO; m];
    }
    if poly.len() == 1 {
        return vec![poly[0]; m];
    }
    let cum = cumulative_lengths(poly);
    let total = cum.last().copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(m);
    let mut i = 0;
    for k in 0..m {
        let s = if m > 1 { total * k as f64 / (m - 1) as f64 } else { 0.0 };
        while i + 2 < cum.len() && cum[i + 1] < s {
            i += 1;
        }
        let seg = (cum[i + 1] - cum[i]).max(1e-12);
        out.push(poly[i].lerp(poly[i + 1], ((s - cum[i]) / seg).clamp(0.0, 1.0)));
    }
    out
}

const SIDE: usize = 65;

/// Four boundary curves (top TL→TR, right TR→BR, bottom BL→BR, left TL→BL), each resampled to
/// [`SIDE`] points, from the outline of a closed path split at the points nearest its bounding
/// box corners.
#[derive(Clone, Debug)]
pub struct Coons {
    top: Vec<Point>,
    right: Vec<Point>,
    bottom: Vec<Point>,
    left: Vec<Point>,
}

impl Coons {
    /// A rectangle.
    pub fn rect(r: Rect) -> Self {
        let line = |a: Point, b: Point| (0..SIDE).map(|i| a.lerp(b, i as f64 / (SIDE - 1) as f64)).collect::<Vec<_>>();
        let (tl, tr, br, bl) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
        Self { top: line(tl, tr), right: line(tr, br), bottom: line(bl, br), left: line(tl, bl) }
    }

    pub fn from_path(path: &PathData) -> Option<Self> {
        // The largest subpath is the outline.
        let sp = path.subpaths.iter().filter(|s| s.anchors.len() >= 2).max_by(|a, b| a.area().abs().total_cmp(&b.area().abs()))?;
        let r = path.bounds()?;
        let tol = (r.width() + r.height()).max(1e-6) / 2000.0;
        let mut poly = flatten_subpath(sp, tol);
        if poly.len() > 1
            && let (Some(a), Some(b)) = (poly.first(), poly.last())
            && a.distance(*b) < 1e-9
        {
            poly.pop();
        }
        if poly.len() < 3 {
            return Some(Self::rect(r));
        }
        // Orient clockwise in y-down (TL → TR → BR → BL).
        let area: f64 = (0..poly.len())
            .map(|i| {
                let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
                p.x * q.y - q.x * p.y
            })
            .sum();
        if area < 0.0 {
            poly.reverse();
        }
        let nearest = |c: Point| (0..poly.len()).min_by(|i, j| poly[*i].distance_squared(c).total_cmp(&poly[*j].distance_squared(c))).unwrap_or(0);
        let tl = nearest(Point::new(r.x0, r.y0));
        let tr = nearest(Point::new(r.x1, r.y0));
        let br = nearest(Point::new(r.x1, r.y1));
        let bl = nearest(Point::new(r.x0, r.y1));
        let n = poly.len();
        let arc = |i: usize, j: usize| -> Vec<Point> {
            let mut v = vec![poly[i]];
            let mut k = i;
            while k != j {
                k = (k + 1) % n;
                v.push(poly[k]);
            }
            v
        };
        let top = resample(&arc(tl, tr), SIDE);
        let right = resample(&arc(tr, br), SIDE);
        let mut bottom = resample(&arc(br, bl), SIDE);
        bottom.reverse();
        let mut left = resample(&arc(bl, tl), SIDE);
        left.reverse();
        Some(Self { top, right, bottom, left })
    }

    /// The point at `u` along a side; past its ends the end segments go on straight (art an
    /// envelope bends can reach past the content's box: outlined strokes).
    fn sample(side: &[Point], u: f64) -> Point {
        let s = u * (side.len() - 1) as f64;
        let i = (s.floor().max(0.0) as usize).min(side.len() - 2);
        side[i].lerp(side[i + 1], s - i as f64)
    }

    /// The surface point at (u, v) ∈ [0,1]².
    pub fn eval(&self, u: f64, v: f64) -> Point {
        let (t, b, l, r) = (Self::sample(&self.top, u), Self::sample(&self.bottom, u), Self::sample(&self.left, v), Self::sample(&self.right, v));
        let (p00, p10, p01, p11) = (self.top[0], self.top[SIDE - 1], self.bottom[0], self.bottom[SIDE - 1]);
        let c = (1.0 - v) * t.to_vec2() + v * b.to_vec2() + (1.0 - u) * l.to_vec2() + u * r.to_vec2()
            - ((1.0 - u) * (1.0 - v) * p00.to_vec2() + u * (1.0 - v) * p10.to_vec2() + (1.0 - u) * v * p01.to_vec2() + u * v * p11.to_vec2());
        c.to_point()
    }

    /// The four corners TL, TR, BR, BL.
    pub fn corners(&self) -> [Point; 4] {
        [self.top[0], self.top[SIDE - 1], self.bottom[SIDE - 1], self.bottom[0]]
    }
}

/// Optional hook that converts nodes the pure evaluation can't map (text) to outlines.
pub type Outliner<'a> = Option<&'a dyn Fn(&Node) -> Option<Node>>;

/// Distort Pattern Fills: the tiles pattern swatch `name` painted with placement `xf` lays over
/// `region` (see [`crate::PatternDef::instances_in`]); `None` without such a swatch.
pub type PatternTiles<'a> = &'a dyn Fn(&str, Affine, Rect) -> Option<Vec<Node>>;

/// What live evaluation asks of the crates above this one (`vectorcraft-effects` supplies them).
#[derive(Clone, Copy, Default)]
pub struct Hooks<'a> {
    /// Type as its glyph outlines, a symbol instance as its art (see [`Outliner`]).
    pub outline: Outliner<'a>,
    /// Distort Appearance: a path's strokes and geometry effects baked into filled art (`None`
    /// when it has none to bake).
    pub appearance: Outliner<'a>,
    /// Distort Pattern Fills: the tiles a pattern fill lays (see [`PatternTiles`]).
    pub pattern: Option<PatternTiles<'a>>,
}

impl<'a> From<Outliner<'a>> for Hooks<'a> {
    fn from(outline: Outliner<'a>) -> Self {
        Self { outline, ..Default::default() }
    }
}

/// The affine map that best matches `f` near `p` (finite differences with step `eps`).
pub fn affine_near(f: &dyn Fn(Point) -> Point, p: Point, eps: f64) -> Affine {
    let o = f(p);
    let ex = (f(p + Vec2::new(eps, 0.0)) - f(p - Vec2::new(eps, 0.0))) / (2.0 * eps);
    let ey = (f(p + Vec2::new(0.0, eps)) - f(p - Vec2::new(0.0, eps))) / (2.0 * eps);
    let lin = Affine::new([ex.x, ex.y, ey.x, ey.y, 0.0, 0.0]);
    Affine::translate(o.to_vec2()) * lin * Affine::translate(-p.to_vec2())
}

/// How deep hooks may nest art inside art (symbols in symbols, envelopes in envelopes) before
/// the rest is left as it is: a file can make a symbol contain itself.
const MAX_HOOK_DEPTH: u32 = 16;

/// How an envelope pushes art through its point map.
#[derive(Clone, Copy)]
struct Warper<'a> {
    f: &'a dyn Fn(Point) -> Point,
    /// Longest piece a segment is cut into before mapping.
    piece: f64,
    /// Finite-difference step for the local affine maps gradients follow.
    eps: f64,
    /// Rows and columns of the pieces an image is cut into.
    cells: usize,
    hooks: Hooks<'a>,
    options: EnvelopeOptions,
    /// How many hooks the art being mapped came through.
    depth: u32,
}

impl Warper<'_> {
    /// This warper one hook deeper (`None` past [`MAX_HOOK_DEPTH`]).
    fn deeper(&self) -> Option<Self> {
        (self.depth < MAX_HOOK_DEPTH).then_some(Self { depth: self.depth + 1, ..*self })
    }
}

/// `to` (art standing in for `from`: its outlines, its art) with `from`'s name, transparency,
/// opacity mask and effects.
fn carry_look(from: &Node, mut to: Node) -> Node {
    to.name = from.name.clone();
    to.opacity = from.opacity;
    to.blend = from.blend;
    to.isolate = from.isolate;
    to.knockout = from.knockout;
    to.knockout_shape = from.knockout_shape;
    to.mask = from.mask.clone();
    to.appearance.effects = from.appearance.effects.clone();
    to
}

fn map_node(n: &Node, w: &Warper) -> Node {
    let deeper = w.deeper();
    // Distort Appearance: strokes and geometry effects bend with the art.
    if w.options.distort_appearance
        && let Some(d) = &deeper
        && let Some(o) = w.hooks.appearance.and_then(|h| h(n))
    {
        return map_node(&o, d);
    }
    // Distort Pattern Fills: the tiles bend with the art (not the patterns inside them).
    if w.options.patterns()
        && let Some(d) = &deeper
        && let Some(o) = w.hooks.pattern.and_then(|p| pattern_art(n, p))
    {
        let inner = Warper { options: EnvelopeOptions { distort_pattern_fills: false, ..d.options }, ..*d };
        return map_node(&o, &inner);
    }
    let mut out = n.clone();
    // Distort Linear Gradients: the gradient vectors follow the warp where they sit.
    if w.options.gradients()
        && let Some(b) = n.geometric_bounds()
    {
        out.appearance.warp_linear_gradients(b, &|p| affine_near(w.f, p, w.eps));
    }
    if let Some(m) = &mut out.mask {
        m.art = Arc::new(map_node(&m.art, w));
    }
    match &mut out.kind {
        NodeKind::Path { path, live, .. } => {
            *path = map_nonlinear(path, w.piece, w.f);
            *live = None;
        }
        NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                *c = Arc::new(map_node(c, w));
            }
        }
        NodeKind::Mesh(m) => m.map(w.f),
        NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => {
            if let Some(d) = &deeper {
                return map_node(&expanded_group_hooks(n, w.hooks), d);
            }
        }
        NodeKind::Text(_) | NodeKind::SymbolInstance { .. } => {
            if let Some(d) = &deeper
                && let Some(o) = w.hooks.outline.and_then(|h| h(n))
            {
                return map_node(&carry_look(n, o), d);
            }
        }
        NodeKind::Image(im) => {
            let im = im.clone();
            return warp_image(&out, &im, w);
        }
        // Its art is plain, so it warps at this depth.
        NodeKind::PlacedDocument(_) => return map_node(&expanded_group_hooks(n, w.hooks), w),
    }
    out
}

/// The affine map taking triangle `s` to triangle `d` (`None` when `s` is degenerate).
fn affine_between(s: [Point; 3], d: [Point; 3]) -> Option<Affine> {
    let basis = |p: [Point; 3]| Affine::new([p[1].x - p[0].x, p[1].y - p[0].y, p[2].x - p[0].x, p[2].y - p[0].y, p[0].x, p[0].y]);
    let bs = basis(s);
    if bs.determinant().abs() < 1e-12 {
        return None;
    }
    let a = basis(d) * bs.inverse();
    a.is_finite().then_some(a)
}

/// Image `n` (`im`) pushed through the warp as a raster mesh warp: its frame cut into
/// `w.cells`² cells of two triangles, each a clip group showing the image under the affine map
/// that takes the triangle where the warp takes its corners. Each piece reaches a little past its
/// edges (at most half a point) so neighbours overlap and no seams show.
fn warp_image(n: &Node, im: &ImageObject, w: &Warper) -> Node {
    let k = w.cells.max(1);
    let (pw, ph) = (im.width as f64, im.height as f64);
    let at = |i: usize, j: usize| Point::new(pw * i as f64 / k as f64, ph * j as f64 / k as f64);
    let mut pieces = Vec::with_capacity(2 * k * k);
    for j in 0..k {
        for i in 0..k {
            let s = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            let d = s.map(|p| (w.f)(im.xf * p));
            for t in [[0, 1, 2], [0, 2, 3]] {
                let (st, dt) = (t.map(|v| s[v]), t.map(|v| d[v]));
                let Some(xf) = affine_between(st, dt) else { continue };
                let c = Point::new((dt[0].x + dt[1].x + dt[2].x) / 3.0, (dt[0].y + dt[1].y + dt[2].y) / 3.0);
                let grow = (dt[0].distance(dt[1]).max(dt[1].distance(dt[2])) * 0.02).min(0.5);
                let tri: Vec<Point> = dt
                    .iter()
                    .map(|p| {
                        let v = *p - c;
                        let len = v.hypot();
                        if len > 1e-12 { *p + v * (grow / len) } else { *p }
                    })
                    .collect();
                let mut clip = Node::path(n.id, PathData::single(SubPath::polyline(&tri, true)), Appearance::default());
                if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                    *clipping = true;
                }
                let image = Node::new(n.id, NodeKind::Image(ImageObject { xf, ..im.clone() }));
                pieces.push(Arc::new(Node::new(n.id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(image)], clip: true })));
            }
        }
    }
    let mut g = n.clone();
    g.kind = NodeKind::Group { children: pieces, clip: false };
    g
}

/// Distort Pattern Fills: path or compound path `n` with each visible pattern fill turned into the
/// tiles it paints in a clip group shaped like `n` (with the fill's opacity and blend mode), its
/// other fills and strokes on copies of it, in paint order, under `n`'s transparency. `None`
/// without a pattern fill whose definition `pattern` finds.
fn pattern_art(n: &Node, tiles: PatternTiles) -> Option<Node> {
    let (path, rule, _) = crate::blend::node_path(n)?;
    let pattern_fill = |i: &AppearanceItem| matches!(i, AppearanceItem::Fill(f) if f.visible && matches!(f.paint, Paint::Pattern { .. }));
    if !n.appearance.items.iter().any(pattern_fill) {
        return None;
    }
    let bounds = path.bounds()?;
    let def = |i: &AppearanceItem| match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Pattern { pattern: name, xf } => tiles(name, *xf, bounds).map(|t| (t, f.opacity, f.blend)),
            _ => None,
        },
        _ => None,
    };
    let plain = |items: Vec<AppearanceItem>| {
        Arc::new(Node {
            appearance: Appearance { items, ..Default::default() },
            opacity: 1.0,
            blend: Default::default(),
            isolate: false,
            knockout: Default::default(),
            knockout_shape: false,
            mask: None,
            name: None,
            ..n.clone()
        })
    };
    let mut pieces = vec![];
    let mut rest = vec![];
    for i in &n.appearance.items {
        let Some((tiles, opacity, blend)) = def(i) else {
            rest.push(i.clone());
            continue;
        };
        if !rest.is_empty() {
            pieces.push(plain(std::mem::take(&mut rest)));
        }
        let mut clip = Node::path(n.id, path.clone(), Appearance::default());
        if let NodeKind::Path { clipping, rule: r, .. } = &mut clip.kind {
            (*clipping, *r) = (true, rule);
        }
        let children = std::iter::once(clip).chain(tiles).map(Arc::new).collect();
        let mut g = Node::new(n.id, NodeKind::Group { children, clip: true });
        g.opacity = opacity;
        g.blend = blend;
        pieces.push(Arc::new(g));
    }
    if !rest.is_empty() {
        pieces.push(plain(rest));
    }
    Some(Node {
        kind: NodeKind::Group { children: pieces, clip: false },
        appearance: Appearance { effects: n.appearance.effects.clone(), ..Default::default() },
        ..n.clone()
    })
}

/// Evaluate an envelope of `kind` around `content` on the page axes with the options envelopes
/// had before Envelope Options (see [`expand_live_hooks`] for an envelope node).
pub fn envelope_expand(content: &[Arc<Node>], kind: &EnvelopeKind, fidelity: f64, outline: Outliner) -> Vec<Node> {
    expand_envelope(content, kind, fidelity, EnvelopeOptions::default(), Affine::IDENTITY, outline.into())
}

/// An envelope's content pushed through its map: Fidelity sets how finely curves are cut (and
/// into how many pieces images are), the options what bends with the art.
fn expand_envelope(content: &[Arc<Node>], kind: &EnvelopeKind, fidelity: f64, options: EnvelopeOptions, frame: Affine, hooks: Hooks) -> Vec<Node> {
    let Some(map) = EnvelopeMap::new(content, kind, frame) else { return content.iter().map(|c| (**c).clone()).collect() };
    let src = map.src();
    let fid = if fidelity.is_finite() { fidelity.clamp(0.0, 100.0) / 100.0 } else { 0.5 };
    let f = |p: Point| map.map(p);
    let w = Warper {
        f: &f,
        piece: src.width().hypot(src.height()).max(1e-6) / (8.0 + fid * 56.0),
        eps: (src.width().max(src.height()) * 0.05).max(0.5),
        cells: (4.0 + fid * 12.0).round() as usize,
        hooks,
        options,
        depth: 0,
    };
    content.iter().map(|c| map_node(c, &w)).collect()
}

/// Envelope bounds: a mesh's points, a top object's path, the warp's surface (sampled).
pub fn envelope_bounds(content: &[Arc<Node>], kind: &EnvelopeKind, frame: Affine) -> Option<Rect> {
    match kind {
        EnvelopeKind::Mesh { rows, cols, points, handles } if !handles.is_empty() => envelope_grid(*rows, *cols, points, handles)?.outline().bounds(),
        EnvelopeKind::Mesh { points, .. } => {
            let first = *points.first()?;
            Some(points.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p)))
        }
        EnvelopeKind::TopObject { path } => path.bounds(),
        EnvelopeKind::Warp { .. } => EnvelopeMap::new(content, kind, frame)?.bounds(),
    }
}

// =====================================================================================
// Expansion
// =====================================================================================

/// Is this one of the live kinds?
pub fn is_live(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::PlacedDocument(_))
}

/// Mesh tessellation as filled quad paths (no stroke), `n`×`n` per patch.
pub fn mesh_quad_nodes(m: &GradientMesh, n: usize) -> Vec<Node> {
    m.quads(n)
        .into_iter()
        .map(|q| {
            let mut ap = Appearance::default();
            let mut fl = FillLayer::new(Paint::solid(q.color));
            fl.opacity = q.opacity;
            ap.items.push(AppearanceItem::Fill(fl));
            Node::path(NodeId(0), PathData::single(SubPath::polyline(&q.pts, true)), ap)
        })
        .collect()
}

/// One level of evaluation of a live object (non-live nodes return themselves). Blend steps and
/// envelope results may still contain live objects (e.g. a blend of two meshes).
pub fn expand_live(n: &Node) -> Vec<Node> {
    expand_live_with(n, None)
}

pub fn expand_live_with(n: &Node, outline: Outliner) -> Vec<Node> {
    expand_live_hooks(n, outline.into())
}

/// [`expand_live_with`] with everything an envelope can ask of the crates above (see [`Hooks`]).
pub fn expand_live_hooks(n: &Node, hooks: Hooks) -> Vec<Node> {
    match &n.kind {
        NodeKind::Blend { children, spec } => blend_expand(children, spec),
        NodeKind::Envelope { content, kind, fidelity, options, frame, .. } => expand_envelope(content, kind, *fidelity, *options, *frame, hooks),
        NodeKind::Mesh(m) => mesh_quad_nodes(m, 8),
        NodeKind::Repeat(r) => r.expand(),
        NodeKind::PlacedDocument(p) => p.art(n),
        _ => vec![n.clone()],
    }
}

/// A live object evaluated into a plain group (keeping the object's id, name and transparency).
pub fn expanded_group(n: &Node, outline: Outliner) -> Node {
    expanded_group_hooks(n, outline.into())
}

/// [`expanded_group`] with [`Hooks`].
pub fn expanded_group_hooks(n: &Node, hooks: Hooks) -> Node {
    let children = expand_live_hooks(n, hooks).into_iter().map(Arc::new).collect();
    let mut g = Node::new(n.id, NodeKind::Group { children, clip: false });
    g.name = n.name.clone();
    g.visible = n.visible;
    g.locked = n.locked;
    g.opacity = n.opacity;
    g.blend = n.blend;
    g.isolate = n.isolate;
    g.knockout = n.knockout;
    // Knocking out opaque blend steps changes nothing: don't make the outputs work for it.
    if matches!(n.kind, NodeKind::Blend { .. }) && g.knockout == crate::Knockout::On && !crate::blend::knockout_shows(&g) {
        g.knockout = crate::Knockout::Off;
    }
    g.knockout_shape = n.knockout_shape;
    g.mask = n.mask.clone();
    g
}

/// Recursively replace every live object in `n` by plain geometry (for exporters).
pub fn expand_deep(n: &Node, outline: Outliner) -> Node {
    expand_deep_hooks(n, outline.into())
}

/// [`expand_deep`] with [`Hooks`].
pub fn expand_deep_hooks(n: &Node, hooks: Hooks) -> Node {
    let mut out = if is_live(n) { expanded_group_hooks(n, hooks) } else { n.clone() };
    if let Some(ch) = out.children_mut() {
        for c in ch.iter_mut() {
            if subtree_has_live(c) {
                *c = Arc::new(expand_deep_hooks(c, hooks));
            }
        }
    }
    out
}

/// Does `n` or any descendant need live evaluation?
pub fn subtree_has_live(n: &Node) -> bool {
    let mut any = false;
    n.walk(&mut |c| any |= is_live(c));
    any
}

/// Largest stroke outset in a subtree (for visual bounds of envelopes).
pub(crate) fn max_outset(nodes: &[Arc<Node>]) -> f64 {
    let mut o = 0.0f64;
    for n in nodes {
        n.walk(&mut |c| o = o.max(c.appearance.outset()));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::shapes;

    fn rect_node(x: f64, y: f64, w: f64, h: f64, c: Color) -> Arc<Node> {
        Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(x, y, x + w, y + h)), Appearance::basic(Paint::solid(c), Paint::None, 0.0)))
    }

    #[test]
    fn lerp_path_equalises_anchor_counts() {
        let a = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0));
        let t = lerp_path(&a, &shapes::polygon(Point::new(5.0, 5.0), 5.0, 7, 0.0), 0.5);
        assert_eq!(t.subpaths[0].anchors.len(), 7);
        let m = lerp_path(&a, &b, 0.0);
        assert_eq!(m.subpaths[0].anchors.len(), 4);
    }

    #[test]
    fn blend_steps_and_colours() {
        let keys = vec![rect_node(0.0, 0.0, 10.0, 10.0, Color::BLACK), rect_node(100.0, 0.0, 10.0, 10.0, Color::WHITE)];
        let out = blend_expand(&keys, &BlendSpec { spacing: BlendSpacing::Steps(3), ..Default::default() });
        assert_eq!(out.len(), 5);
        let mid = out[2].appearance.fill_paint().color().unwrap().to_rgb()[0];
        assert!((mid - 0.5).abs() < 1e-3);
    }

    #[test]
    fn coons_rect_is_identity() {
        let c = Coons::from_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0))).unwrap();
        let p = c.eval(0.25, 0.5);
        assert!((p.x - 25.0).abs() < 1e-6 && (p.y - 25.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn maps_go_on_past_the_content_box() {
        // A stroke outline reaches past the box: a flat mesh and a rectangle stay the identity there.
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let p = grid_eval(2, 2, &grid_points(r, 2, 2), -0.05, 1.1);
        assert!((p.x + 5.0).abs() < 1e-6 && (p.y - 110.0).abs() < 1e-6, "{p:?}");
        let c = Coons::from_path(&shapes::rectangle(r)).unwrap().eval(1.05, -0.1);
        assert!((c.x - 105.0).abs() < 1e-6 && (c.y + 10.0).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn grid_identity() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let pts = grid_points(r, 3, 4);
        let p = grid_eval(3, 4, &pts, 0.37, 0.81);
        assert!((p.x - 37.0).abs() < 1e-6 && (p.y - 81.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn mesh_insert_and_remove_lines() {
        let mut m =
            GradientMesh::for_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        let i = m.add_lines_at(Point::new(30.0, 60.0)).unwrap();
        assert_eq!((m.rows, m.cols), (2, 2));
        assert!(m.points[i].p.distance(Point::new(30.0, 60.0)) < 0.5, "{:?}", m.points[i].p);
        assert!(m.remove_point_lines(i));
        assert_eq!((m.rows, m.cols), (1, 1));
    }
}
