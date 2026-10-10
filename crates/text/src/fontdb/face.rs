//! Loaded font face data, metrics, glyph coverage and outline access.

use super::{FontBytes, FontFace, GlyphId, IcfMargins, LocationRef, MetadataProvider, Size, Variation, base_values};

impl FontFace {
    pub(crate) fn data(&self) -> &[u8] {
        match &self.bytes {
            FontBytes::Static(b) => b,
            FontBytes::Owned(v) => v.as_slice(),
        }
    }

    pub(crate) fn skrifa(&self) -> Option<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(self.data(), self.index).ok()
    }

    pub(crate) fn hb(&self) -> Option<harfrust::FontRef<'_>> {
        harfrust::FontRef::from_index(self.data(), self.index).ok()
    }

    /// The face's index in its font file (collections hold several).
    pub(crate) fn index(&self) -> u32 {
        self.index
    }

    /// Where in a variable font's design space the face's outlines and metrics are taken (the
    /// default location for a static face).
    pub(crate) fn location(&self) -> LocationRef<'_> {
        LocationRef::from(&self.location)
    }

    /// The axis settings of the variable font's named instance this face is (`wght` 700 for
    /// Bold), in user units; empty for a static face and a variable font's default instance.
    /// The face's file holds the whole variable font: exporters that embed it pass these on.
    pub fn variations(&self) -> &[Variation] {
        &self.variations
    }

    /// Unique id of this face within the process.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Does the face map `c` to a glyph?
    pub fn covers(&self, c: char) -> bool {
        self.skrifa().is_some_and(|f| f.charmap().map(c).is_some())
    }

    /// Units per em.
    pub fn units_per_em(&self) -> f64 {
        self.upem
    }

    /// (ascent, descent) in font units, both positive.
    pub fn vertical_metrics(&self) -> (f64, f64) {
        (self.ascent, self.descent)
    }

    /// Every mapped character and its glyph id, sorted by code point (the Glyphs panel).
    pub fn chars(&self) -> Vec<(char, u32)> {
        let Some(f) = self.skrifa() else { return vec![] };
        let mut v: Vec<(char, u32)> = f.charmap().mappings().filter_map(|(cp, g)| char::from_u32(cp).map(|c| (c, g.to_u32()))).collect();
        v.sort_unstable_by_key(|x| x.0);
        v.dedup_by_key(|x| x.0);
        v
    }

    /// Advance width of glyph `gid` in font units.
    pub fn advance(&self, gid: u32) -> f64 {
        self.skrifa()
            .and_then(|f| f.glyph_metrics(Size::unscaled(), self.location()).advance_width(GlyphId::new(gid)))
            .map(|a| a as f64)
            .unwrap_or(self.upem * 0.5)
    }

    /// Glyph `gid` set upright in vertical type, from the font's vertical metrics: (its advance down
    /// the column, the height above the baseline of its vertical origin, the top of its cell), in
    /// font units. The origin is the `VORG` table's, else the glyph's top plus its top side bearing.
    /// `None` when the font has no vertical metrics (`vhea`/`vmtx`), or they make no sense.
    pub fn vertical_glyph(&self, gid: u32) -> Option<(f64, f64)> {
        use skrifa::raw::TableProvider;
        let f = self.skrifa()?;
        let vmtx = f.vmtx().ok()?;
        let g = GlyphId::new(gid);
        let advance = f64::from(vmtx.advance(g)?);
        let origin = match f.vorg() {
            Ok(vorg) => f64::from(vorg.vertical_origin_y(g)),
            Err(_) => {
                let top = f.glyph_metrics(Size::unscaled(), self.location()).bounds(g).map_or(self.ascent, |b| f64::from(b.y_max));
                top + f64::from(vmtx.side_bearing(g)?)
            }
        };
        let em = self.upem;
        // A glyph's cell is somewhere between a tenth of an em and a few ems, its top within a few
        // ems of the baseline: anything else is a damaged table.
        (advance > em * 0.1 && advance < em * 4.0 && origin.abs() < em * 4.0).then_some((advance, origin))
    }

    /// Height above the baseline of the centre of the face's ideographic em box, in ems: from its
    /// vertical metrics (an ideograph's cell), else the usual 0.38 (the box running from 0.12 em
    /// below the baseline to 0.88 em above it).
    pub fn ideographic_centre(&self) -> f64 {
        *self.ideographic_centre.get_or_init(|| {
            let gid = ['国', 'あ', '一'].into_iter().map(|c| self.glyph_for(c)).find(|g| *g != 0);
            gid.and_then(|g| self.vertical_glyph(g)).map_or(0.38, |(advance, origin)| (origin - advance * 0.5) / self.upem)
        })
    }

    /// The ideographic character face as margins inside the em box ([`IcfMargins`]): from the
    /// font's BASE table as the OpenType baseline tags define it (`icfb`, `icft` against `ideo`,
    /// `idtp`), else, as that definition allows, from the bounds of some ideographs and kana
    /// averaged, else none (the ICF is the em box).
    /// <https://learn.microsoft.com/en-us/typography/opentype/spec/baselinetags>
    pub fn icf_margins(&self) -> IcfMargins {
        *self.icf_margins.get_or_init(|| self.icf_from_base().or_else(|| self.icf_from_glyphs()).unwrap_or_default())
    }

    /// [`Self::icf_margins`] from the BASE table: the ICF's bottom edge `icfb` (top edge `icft`, else
    /// as far below the em box top) against the em box `ideo` .. `idtp` (else the OS/2 typographic
    /// descender, and one em above it); across vertical lines, `icfb` and `icft` of the vertical
    /// axis against 0 .. `idtp` (else the horizontal margin, and one em).
    pub(crate) fn icf_from_base(&self) -> Option<IcfMargins> {
        use skrifa::raw::TableProvider;
        let f = self.skrifa()?;
        let base = f.base().ok()?;
        let em = self.upem;
        let h = base_values(base.horiz_axis()?.ok()?)?;
        let icfb = *h.get(b"icfb")?;
        let em_bottom = match h.get(b"ideo") {
            Some(v) => *v,
            None => f64::from(f.os2().ok()?.s_typo_descender()),
        };
        let em_top = h.get(b"idtp").copied().unwrap_or(em_bottom + em);
        let margin = icfb - em_bottom;
        let icf_top = h.get(b"icft").copied().unwrap_or(em_top - margin);
        let v = base.vert_axis().and_then(Result::ok).and_then(base_values).unwrap_or_default();
        let icf_left = v.get(b"icfb").copied().unwrap_or(margin);
        let em_right = v.get(b"idtp").copied().unwrap_or(em);
        let icf_right = v.get(b"icft").copied().unwrap_or(em_right - icf_left);
        let m = IcfMargins { top: (em_top - icf_top) / em, bottom: margin / em, right: (em_right - icf_right) / em, left: icf_left / em };
        // Inside the em box, short of its middle: anything else is a damaged table.
        [m.top, m.bottom, m.right, m.left].iter().all(|x| (0.0..0.5).contains(x)).then_some(m)
    }

    /// [`Self::icf_margins`] from the ink of some ideographs and kana: each edge's margin to the em
    /// box (its centre [`Self::ideographic_centre`], one em high and wide), averaged.
    pub(crate) fn icf_from_glyphs(&self) -> Option<IcfMargins> {
        let f = self.skrifa()?;
        let metrics = f.glyph_metrics(Size::unscaled(), self.location());
        let em = self.upem;
        let (em_bottom, em_top) = ((self.ideographic_centre() - 0.5) * em, (self.ideographic_centre() + 0.5) * em);
        let boxes: Vec<_> = ['国', '東', '永', '書', 'あ', 'ア']
            .into_iter()
            .map(|c| self.glyph_for(c))
            .filter(|g| *g != 0)
            .filter_map(|g| metrics.bounds(GlyphId::new(g)))
            .collect();
        if boxes.is_empty() {
            return None;
        }
        let n = boxes.len() as f64;
        let mean = |v: &dyn Fn(&skrifa::metrics::BoundingBox) -> f32| boxes.iter().map(|b| f64::from(v(b))).sum::<f64>() / n;
        let m = IcfMargins {
            top: (em_top - mean(&|b| b.y_max)) / em,
            bottom: (mean(&|b| b.y_min) - em_bottom) / em,
            right: (em - mean(&|b| b.x_max)) / em,
            left: mean(&|b| b.x_min) / em,
        };
        [m.top, m.bottom, m.right, m.left].iter().all(|x| (0.0..0.5).contains(x)).then_some(m)
    }

    /// Glyph id for `c` (0 = .notdef).
    pub fn glyph_for(&self, c: char) -> u32 {
        self.skrifa().and_then(|f| f.charmap().map(c)).map(|g| g.to_u32()).unwrap_or(0)
    }

    /// The file the face was read from (cataloged system fonts).
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// The bytes of the face's font file (a collection holds several faces).
    pub fn file_data(&self) -> &[u8] {
        self.data()
    }

    /// The embedding permissions of the OS/2 table (`fsType`; 0, installable, when it has none).
    pub fn fs_type(&self) -> u16 {
        use skrifa::raw::TableProvider;
        self.skrifa().and_then(|f| f.os2().ok()).map_or(0, |t| t.fs_type())
    }

    /// May the font file be copied along with a document? Its license allows embedding its
    /// outlines, as exports check it ([`Self::embedding`]).
    pub fn embeddable(&self) -> bool {
        self.embedding() != crate::embed::Embedding::Forbidden
    }

    /// The face's index in its font file ([`Self::file_data`]; collections hold several).
    pub fn face_index(&self) -> u32 {
        self.index
    }
}
