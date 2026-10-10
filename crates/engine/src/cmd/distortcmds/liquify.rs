//! Liquify stroke sampling, target ownership and geometry edits.

use super::super::*;
use super::{Arc, Dabber, Document, LiquifyParams, Node, NodeId, NodeKind, PathData, PathStroke, Point, Rect, Sample, Value, json, reach_bounds};

// ---------- liquify ----------

pub(super) fn points_of(p: &Value, key: &str) -> Option<Vec<Point>> {
    p.get(key)?.as_array()?.iter().map(|q| Some(Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect()
}

/// A stroke's `[[x, y, pressure?]…]` samples (pressure 0..1, default 1).
fn samples_of(p: &Value, key: &str) -> Option<Vec<Sample>> {
    p.get(key)?
        .as_array()?
        .iter()
        .map(|q| Some((Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?), q.get(2).map_or(Some(1.0), Value::as_f64)?)))
        .collect()
}

/// What a liquify stroke may reach: the paths it can edit, each with the box no dab outside
/// changes it ([`reach_bounds`]), and the objects it leaves as they are (type, symbols, images,
/// graphs, meshes, and envelopes, repeats and blends with their contents), each with its bounds
/// and what it is. Hidden and locked objects, and guides, are neither.
#[derive(Default)]
struct Targets {
    paths: Vec<(NodeId, Rect)>,
    blocked: Vec<(NodeId, Rect, &'static str)>,
    /// Every object listed (selected roots may hold each other).
    seen: std::collections::HashSet<NodeId>,
}

/// What `n` is when Liquify can't distort it (and doesn't look inside it).
fn not_liquified(n: &Node) -> Option<&'static str> {
    Some(match &n.kind {
        NodeKind::Group { .. } if n.graph.is_some() => "graphs",
        NodeKind::Text(_) => "type",
        NodeKind::SymbolInstance { .. } => "symbols",
        NodeKind::Image(_) => "images",
        NodeKind::Mesh(_) => "meshes",
        NodeKind::Envelope { .. } => "envelopes",
        NodeKind::Repeat(_) => "repeats",
        NodeKind::PlacedDocument(_) => "placed documents",
        NodeKind::Blend { .. } => "blends",
        _ => return None,
    })
}

impl Targets {
    /// The targets under `roots` (the selection), or with none every visible object.
    fn of(d: &Document, roots: &[NodeId]) -> Self {
        let mut t = Self::default();
        if roots.is_empty() {
            d.layers.iter().for_each(|l| t.visit(l));
            return t;
        }
        for r in roots {
            let (Some(n), Some(chain)) = (d.node(*r), d.ancestry(*r)) else { continue };
            if !d.is_editable(*r) {
                continue;
            }
            // Inside a graph, an envelope, a repeat or a blend: that object is what is left alone.
            match chain.iter().filter_map(|a| d.node(*a)).find_map(|a| Some((a, not_liquified(a)?))) {
                Some((a, what)) => t.block(a, what),
                None => t.visit(n),
            }
        }
        t
    }

    fn block(&mut self, n: &Node, what: &'static str) {
        if let Some(b) = n.geometric_bounds()
            && self.seen.insert(n.id)
        {
            self.blocked.push((n.id, b, what));
        }
    }

    fn visit(&mut self, n: &Node) {
        if !n.visible || n.locked {
            return;
        }
        if let Some(what) = not_liquified(n) {
            return self.block(n, what);
        }
        match &n.kind {
            NodeKind::Path { guide: false, path, .. } => {
                if let Some(b) = reach_bounds(path)
                    && self.seen.insert(n.id)
                {
                    self.paths.push((n.id, b));
                }
            }
            _ => n.children().into_iter().flatten().for_each(|c| self.visit(c)),
        }
    }
}

/// Does a dab with box `bb` reach `b`?
fn reaches(bb: Rect, b: Rect) -> bool {
    b.inflate(1e-6, 1e-6).intersect(bb).area() > 0.0
}

/// A liquify stroke being applied: its dabs and the paths they reached so far, so a longer stroke
/// (the next sample of a drag) applies only the dabs it adds. It belongs to one document snapshot
/// (`base`, held so the pointer can't be reused) and one set of other parameters (`key`).
pub(crate) struct LiquifyStroke {
    base: Arc<Document>,
    key: Value,
    prm: LiquifyParams,
    dabs: Dabber,
    targets: Targets,
    /// The paths reached, by their index in `targets.paths`.
    paths: Vec<(usize, PathStroke)>,
    /// The objects left alone that the brush passed over, by their index in `targets.blocked`.
    skipped: Vec<usize>,
    /// Which of `targets.paths` and `targets.blocked` the brush reached.
    reached: (Vec<bool>, Vec<bool>),
}

impl LiquifyStroke {
    fn new(base: Arc<Document>, key: Value, prm: LiquifyParams, roots: &[NodeId]) -> Self {
        let targets = Targets::of(&base, roots);
        let reached = (vec![false; targets.paths.len()], vec![false; targets.blocked.len()]);
        Self { base, key, prm, dabs: Dabber::new(&prm), targets, paths: vec![], skipped: vec![], reached }
    }

    /// Start the targets dab box `bb` reaches.
    fn reach(&mut self, bb: Rect) {
        for (i, (id, b)) in self.targets.paths.iter().enumerate() {
            if let Some(r) = self.reached.0.get_mut(i).filter(|r| !**r && reaches(bb, *b)) {
                *r = true;
                let Some(path) = self.base.node(*id).and_then(|n| n.path_data()) else { continue };
                self.paths.push((i, PathStroke::new(path.clone(), id.0)));
            }
        }
        for (i, (_, b, _)) in self.targets.blocked.iter().enumerate() {
            if let Some(r) = self.reached.1.get_mut(i).filter(|r| !**r && reaches(bb, *b)) {
                *r = true;
                self.skipped.push(i);
            }
        }
    }

    /// Add `samples` to the stroke and apply the dabs they make.
    fn extend(&mut self, samples: &[Sample]) {
        let from = self.dabs.dabs.len();
        for s in samples {
            self.dabs.push(*s);
        }
        for i in from..self.dabs.dabs.len() {
            if let Some(c) = self.dabs.dabs.get(i).map(|d| d.c) {
                self.reach(self.prm.brush_bounds(c));
            }
        }
        if let Some(t) = self.dabs.tail() {
            self.reach(self.prm.brush_bounds(t.c));
        }
        for (_, ps) in &mut self.paths {
            ps.advance(&self.dabs.dabs, &self.prm);
        }
    }

    /// The paths the stroke changed (in document order) and their new geometry.
    fn results(&self) -> Vec<(NodeId, PathData)> {
        let tail = self.dabs.tail();
        let mut out: Vec<(usize, NodeId, PathData)> =
            self.paths.iter().filter_map(|(i, ps)| Some((*i, self.targets.paths.get(*i)?.0, ps.finish(&self.dabs.dabs, tail, &self.prm)?))).collect();
        out.sort_by_key(|(i, ..)| *i);
        out.into_iter().map(|(_, id, p)| (id, p)).collect()
    }

    /// The objects under the brush left as they are, and the status message about them.
    fn skipped(&self) -> (Vec<u64>, Option<String>) {
        let mut ids = vec![];
        let mut kinds: Vec<&str> = vec![];
        for (id, _, what) in self.skipped.iter().filter_map(|i| self.targets.blocked.get(*i)) {
            ids.push(id.0);
            if !kinds.contains(what) {
                kinds.push(what);
            }
        }
        let msg = (!ids.is_empty()).then(|| {
            let n = ids.len();
            format!(
                "{} left {n} object{} under the brush as {} ({}): it reshapes paths only",
                self.prm.kind.label(),
                if n == 1 { "" } else { "s" },
                if n == 1 { "it was" } else { "they were" },
                kinds.join(", ")
            )
        });
        (ids, msg)
    }
}

pub(super) fn liquify(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.liquify";
    let prm = LiquifyParams::from_json(p).ok_or_else(|| bad(C, "missing or unknown `tool`"))?;
    let pts = samples_of(p, "points").filter(|v| !v.is_empty()).ok_or_else(|| bad(C, "points must be a non-empty [[x,y,pressure?]…] list"))?;
    if pts.iter().any(|(q, f)| !q.x.is_finite() || !q.y.is_finite() || !f.is_finite()) {
        return Err(bad(C, "points must be finite"));
    }
    let roots = match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(v) => v,
        None => s.doc()?.selection.objects.clone(),
    };
    let base = s.doc()?.doc.clone();
    let mut key = p.clone();
    if let Some(o) = key.as_object_mut() {
        o.remove("points");
        o.insert("__roots".into(), json!(roots.iter().map(|r| r.0).collect::<Vec<_>>()));
    }
    // A live drag: the same stroke as the last preview with more samples goes on from there.
    let mut stroke = match s.liquify_stroke.take() {
        Some(st) if Arc::ptr_eq(&st.base, &base) && st.key == key && pts.starts_with(st.dabs.samples()) => st,
        _ => Box::new(LiquifyStroke::new(base, key, prm, &roots)),
    };
    let done = stroke.dabs.samples().len();
    stroke.extend(pts.get(done..).unwrap_or_default());
    let results = stroke.results();
    let (skipped, warning) = stroke.skipped();
    let changed = s.edit(&format!("{} Tool", prm.kind.label()), |d, _| {
        let mut changed = vec![];
        for (id, new) in results {
            let Some(n) = d.node_mut(id) else { continue };
            if let NodeKind::Path { path, live, .. } = &mut n.kind {
                *path = new;
                *live = None;
                changed.push(id.0);
            }
        }
        Ok(changed)
    })?;
    if s.in_interaction() {
        s.liquify_stroke = Some(stroke);
    }
    let mut out = json!({ "ids": changed, "skipped": skipped });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}
