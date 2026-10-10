//! Live repeat geometry and source-instance expansion.

use super::{Affine, Arc, Deserialize, Node, NodeId, NodeKind, Point, Rect, Serialize, TileType, Vec2, full_turn, three, tile_offset, zero_ids};

// =====================================================================================
// Repeat
// =====================================================================================

/// The arrangement of a Repeat object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RepeatKind {
    /// Copies rotated around `center`; the source's centre is kept `radius` from it.
    #[serde(rename_all = "camelCase")]
    Radial {
        instances: u32,
        radius: f64,
        center: Point,
        #[serde(default)]
        reverse_overlap: bool,
        #[serde(default)]
        start_angle: f64,
        #[serde(default = "full_turn")]
        end_angle: f64,
    },
    /// Copies on a grid (tile = source bounds + spacing), optionally brick/hex offset and with
    /// alternate rows/columns flipped.
    #[serde(rename_all = "camelCase")]
    Grid {
        h_spacing: f64,
        v_spacing: f64,
        #[serde(default = "three")]
        rows: u32,
        #[serde(default = "three")]
        cols: u32,
        #[serde(default)]
        grid_type: TileType,
        /// Flip alternate rows vertically.
        #[serde(default)]
        flip_rows: bool,
        /// Flip alternate columns horizontally.
        #[serde(default)]
        flip_cols: bool,
    },
    /// The source plus its reflection across the axis through `center` at `angle` degrees
    /// (90 = vertical axis).
    Mirror { angle: f64, center: Point },
}

impl RepeatKind {
    pub fn label(&self) -> &'static str {
        match self {
            RepeatKind::Radial { .. } => "Radial Repeat",
            RepeatKind::Grid { .. } => "Grid Repeat",
            RepeatKind::Mirror { .. } => "Mirror Repeat",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepeatSpec {
    pub kind: RepeatKind,
    pub source: Vec<Arc<Node>>,
}

/// Reflection across the line through `c` with direction angle `deg`.
pub fn reflect_about(c: Point, deg: f64) -> Affine {
    let t = (2.0 * deg).to_radians();
    let (s2, c2) = t.sin_cos();
    Affine::translate(c.to_vec2()) * Affine::new([c2, s2, s2, -c2, 0.0, 0.0]) * Affine::translate(-c.to_vec2())
}

/// Instance transforms of a repeat (source space → document), in paint order.
pub fn repeat_transforms(kind: &RepeatKind, source_bounds: Option<Rect>) -> Vec<Affine> {
    let b = source_bounds.unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
    match kind {
        RepeatKind::Radial { instances, radius, center, reverse_overlap, start_angle, end_angle } => {
            let n = (*instances).clamp(1, 1000);
            let c0 = b.center();
            let mut d = c0 - *center;
            if d.hypot() < 1e-9 {
                d = Vec2::new(0.0, -1.0);
            }
            let pre = Affine::translate(*center + d.normalize() * radius.max(0.0) - c0);
            let sweep = end_angle - start_angle;
            let full = (sweep.abs() - 360.0).abs() < 1e-9;
            let step = if full || n == 1 { sweep / n as f64 } else { sweep / (n - 1) as f64 };
            let mut v: Vec<Affine> = (0..n)
                .map(|k| {
                    let a = (start_angle + step * k as f64).to_radians();
                    Affine::translate(center.to_vec2()) * Affine::rotate(a) * Affine::translate(-center.to_vec2()) * pre
                })
                .collect();
            if *reverse_overlap {
                v.reverse();
            }
            v
        }
        RepeatKind::Grid { h_spacing, v_spacing, rows, cols, grid_type, flip_rows, flip_cols } => {
            let (w, h) = ((b.width() + h_spacing).max(1e-3), (b.height() + v_spacing).max(1e-3));
            let mut v = vec![];
            for j in 0..(*rows).clamp(1, 500) as i64 {
                for i in 0..(*cols).clamp(1, 500) as i64 {
                    let o = tile_offset(*grid_type, w, h, i, j);
                    let c = b.center();
                    let mut flip = Affine::IDENTITY;
                    if *flip_cols && i % 2 == 1 {
                        flip = reflect_about(c, 90.0) * flip;
                    }
                    if *flip_rows && j % 2 == 1 {
                        flip = reflect_about(c, 0.0) * flip;
                    }
                    v.push(Affine::translate(o) * flip);
                }
            }
            v
        }
        RepeatKind::Mirror { angle, center } => vec![Affine::IDENTITY, reflect_about(*center, *angle)],
    }
}

impl RepeatSpec {
    pub fn source_bounds(&self) -> Option<Rect> {
        crate::live::nodes_bounds(&self.source)
    }
    pub fn transforms(&self) -> Vec<Affine> {
        repeat_transforms(&self.kind, self.source_bounds())
    }
    /// Evaluated instances: one group of the (transformed) source per instance (ids 0).
    pub fn expand(&self) -> Vec<Node> {
        self.transforms()
            .into_iter()
            .map(|m| {
                let children = self
                    .source
                    .iter()
                    .map(|s| {
                        let mut n = (**s).clone();
                        n.transform(m, false);
                        zero_ids(&mut n);
                        Arc::new(n)
                    })
                    .collect();
                Node::group(NodeId(0), children)
            })
            .collect()
    }
    /// Geometric bounds of all instances.
    pub fn bounds(&self) -> Option<Rect> {
        let b = self.source_bounds()?;
        self.transforms().into_iter().fold(None, |acc, m| vectorcraft_geom::union_opt(acc, Some(m.transform_rect_bbox(b))))
    }
    /// Apply a document transform (source + arrangement parameters).
    pub fn transform(&mut self, a: Affine, scaling: impl Into<crate::node::Scaling>) {
        let scaling = scaling.into();
        for c in self.source.iter_mut() {
            Arc::make_mut(c).transform(a, scaling);
        }
        let s = a.determinant().abs().sqrt();
        match &mut self.kind {
            RepeatKind::Radial { radius, center, .. } => {
                *center = a * *center;
                *radius *= s;
            }
            RepeatKind::Grid { h_spacing, v_spacing, .. } => {
                *h_spacing *= s;
                *v_spacing *= s;
            }
            RepeatKind::Mirror { angle, center } => {
                *center = a * *center;
                let d = a * Point::new(angle.to_radians().cos(), angle.to_radians().sin()) - a * Point::ZERO;
                *angle = d.y.atan2(d.x).to_degrees();
            }
        }
    }
    /// Default radial repeat around the source (centre below it, radius = its height × 1.2).
    pub fn radial(source: Vec<Arc<Node>>, instances: u32, radius: Option<f64>) -> Self {
        let b = crate::live::nodes_bounds(&source).unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
        let r = radius.unwrap_or((b.height().max(b.width()) * 1.2).max(10.0));
        let center = b.center() + Vec2::new(0.0, r);
        Self { kind: RepeatKind::Radial { instances, radius: r, center, reverse_overlap: false, start_angle: 0.0, end_angle: 360.0 }, source }
    }
    pub fn grid(source: Vec<Arc<Node>>, h_spacing: f64, v_spacing: f64) -> Self {
        Self {
            kind: RepeatKind::Grid { h_spacing, v_spacing, rows: 3, cols: 3, grid_type: TileType::Grid, flip_rows: false, flip_cols: false },
            source,
        }
    }
    /// Mirror across an axis at `angle` through a point `offset` beyond the source's right edge
    /// (vertical axis) or below it (other angles pass through the right-edge midpoint).
    pub fn mirror(source: Vec<Arc<Node>>, angle: f64, offset: f64) -> Self {
        let b = crate::live::nodes_bounds(&source).unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
        let center = Point::new(b.x1 + offset, b.center().y);
        Self { kind: RepeatKind::Mirror { angle, center }, source }
    }
}

/// Is this a Repeat object?
pub fn is_repeat(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Repeat(_))
}
