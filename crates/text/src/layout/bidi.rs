//! Paragraph bidi analysis and visual run ordering.

use super::{BidiInfo, Level, ParaDirection, Range, SGlyph};

/// The bidirectional resolution (UAX #9) of paragraph `text`, its base direction `direction` or,
/// without one, from its first strong character. `None` for the usual paragraph with nothing right
/// to left in it (no right-to-left character or direction), which skips the algorithm.
pub(crate) fn para_bidi(text: &str, direction: Option<ParaDirection>) -> Option<BidiInfo<'_>> {
    use unicode_bidi::BidiClass::{AL, AN, FSI, R, RLE, RLI, RLO};
    let rtl_set = direction == Some(ParaDirection::RightToLeft);
    if !rtl_set && !text.chars().any(|c| matches!(unicode_bidi::bidi_class(c), R | AL | AN | RLE | RLO | RLI | FSI)) {
        return None;
    }
    let level = direction.map(|d| if d == ParaDirection::RightToLeft { Level::rtl() } else { Level::ltr() });
    Some(BidiInfo::new(text, level))
}

/// Does the paragraph run right to left ([`para_bidi`])?
pub(crate) fn is_rtl(bidi: Option<&BidiInfo<'_>>) -> bool {
    bidi.and_then(|b| b.paragraphs.first()).is_some_and(|p| p.level.is_rtl())
}

/// The visual order of a line's `glyphs` (indices into them), whole shaping clusters reordered
/// (UAX #9 L1–L2) for the line `line` (bytes of the text) of the paragraph starting at byte
/// `paragraph_start`; their own order when nothing is right to left (`bidi` None).
pub(super) fn visual_order(bidi: Option<&BidiInfo<'_>>, paragraph_start: usize, line: Range<usize>, glyphs: &[SGlyph]) -> Vec<usize> {
    let Some((bidi, para)) = bidi.filter(|b| b.has_rtl()).and_then(|b| Some((b, b.paragraphs.first()?))) else {
        return (0..glyphs.len()).collect();
    };
    if glyphs.is_empty() {
        return vec![];
    }
    let levels = bidi.reordered_levels(para, line.start.saturating_sub(paragraph_start)..line.end.saturating_sub(paragraph_start));
    let mut clusters: Vec<Range<usize>> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        if i > 0 && glyphs.get(i - 1).is_some_and(|p| p.byte == g.byte) {
            if let Some(c) = clusters.last_mut() {
                c.end = i + 1;
            }
        } else {
            clusters.push(i..i + 1);
        }
    }
    let cluster_levels: Vec<_> = clusters
        .iter()
        .filter_map(|c| glyphs.get(c.start))
        .map(|g| levels.get(g.byte.saturating_sub(paragraph_start)).copied().unwrap_or(g.level))
        .collect();
    unicode_bidi::BidiInfo::reorder_visual(&cluster_levels).into_iter().filter_map(|i| clusters.get(i)).flat_map(|c| c.clone()).collect()
}
