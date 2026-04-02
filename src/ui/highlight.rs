use ratatui::style::Style;

use crate::ui::theme::EditorColors;

// ── HighlightKind ─────────────────────────────────────────────────────────────

/// The source type of a highlight, used to resolve style and priority.
///
/// Variants are ordered by **ascending priority** — derived `Ord` means later
/// variants win when multiple highlights overlap at the same position. The
/// ordering mirrors the visual importance of each source:
///
/// - `SearchMatch` — broad, can span many characters
/// - `BracketMatch` — single character, must stay visible even inside a match
///
/// Future sources slot in by inserting at the appropriate priority level:
/// - `Syntax` would go *before* `SearchMatch` (lowest priority)
/// - `Diagnostic` would go between `Syntax` and `SearchMatch`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HighlightKind {
    // Variants listed in ascending priority order — do not reorder.
    // Future: Syntax,
    // Future: Diagnostic,
    SearchMatch,
    BracketMatch,
}

impl HighlightKind {
    /// Resolve this kind to its theme color.
    ///
    /// Called by [`resolve_style`](crate::ui::renderer) after the winning kind
    /// has been determined. Producers push kinds; the renderer maps them to
    /// styles — decoupling highlight production from the color scheme.
    pub(crate) fn style(self, colors: &EditorColors) -> Style {
        match self {
            Self::SearchMatch => colors.search_match,
            Self::BracketMatch => colors.bracket_match,
        }
    }
}

// ── HighlightMap ──────────────────────────────────────────────────────────────

/// A per-frame map of document highlights.
///
/// Each entry is an inclusive char range `[start, end]` with a
/// [`HighlightKind`]. Build once per frame (push all entries, then call
/// `build()` to sort), then query per character with `kind_at`.
///
/// Unlike the old `HighlightSet`, this correctly handles overlapping ranges
/// from different sources: `kind_at` scans all entries that contain a position
/// and returns the highest-priority kind. Adding a new highlight source means
/// adding a `HighlightKind` variant and a producer that calls `push` — the
/// renderer and priority logic require no changes.
pub(crate) struct HighlightMap {
    /// Sorted by `start` after `build()` is called.
    entries: Vec<(usize, usize, HighlightKind)>,
}

/// A permanent empty map for tests and contexts where no highlights are needed.
/// Avoids a heap allocation and can be stored as a `&'static`.
#[cfg(test)]
pub(crate) static EMPTY: HighlightMap = HighlightMap { entries: Vec::new() };

impl HighlightMap {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub(crate) fn push(&mut self, start: usize, end: usize, kind: HighlightKind) {
        self.entries.push((start, end, kind));
    }

    /// Sort entries by start position so `kind_at` can binary-search.
    /// Call once after all `push` calls, before querying.
    pub(crate) fn build(mut self) -> Self {
        self.entries.sort_unstable_by_key(|&(s, _, _)| s);
        self
    }

    /// Return the highest-priority [`HighlightKind`] at `pos`, if any.
    ///
    /// Requires `build()` to have been called. Binary-searches for the last
    /// entry with `start ≤ pos`, then scans backwards collecting all entries
    /// that contain `pos` (end ≥ pos). Returns the maximum kind — i.e. the one
    /// with the highest priority per the `HighlightKind` ordering.
    ///
    /// Handles overlapping ranges correctly: if a search match and a bracket
    /// match coincide at the same character, `BracketMatch` wins.
    pub(crate) fn kind_at(&self, pos: usize) -> Option<HighlightKind> {
        let limit = self.entries.partition_point(|&(s, _, _)| s <= pos);
        self.entries[..limit]
            .iter()
            .rev()
            .filter(|&&(_, end, _)| pos <= end)
            .map(|&(_, _, kind)| kind)
            .max()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn map(ranges: &[(usize, usize, HighlightKind)]) -> HighlightMap {
        let mut m = HighlightMap::new();
        for &(s, e, kind) in ranges {
            m.push(s, e, kind);
        }
        m.build()
    }

    // ── kind_at: basic ────────────────────────────────────────────────────────

    #[test]
    fn empty_returns_none() {
        assert_eq!(EMPTY.kind_at(0), None);
        assert_eq!(EMPTY.kind_at(100), None);
    }

    #[test]
    fn single_point_range() {
        let m = map(&[(5, 5, HighlightKind::BracketMatch)]);
        assert_eq!(m.kind_at(4), None);
        assert_eq!(m.kind_at(5), Some(HighlightKind::BracketMatch));
        assert_eq!(m.kind_at(6), None);
    }

    #[test]
    fn pos_before_all_ranges() {
        let m = map(&[(5, 10, HighlightKind::SearchMatch)]);
        assert_eq!(m.kind_at(0), None);
        assert_eq!(m.kind_at(4), None);
        assert_eq!(m.kind_at(5), Some(HighlightKind::SearchMatch));
    }

    #[test]
    fn adjacent_ranges_do_not_bleed() {
        let m = map(&[
            (0, 2, HighlightKind::SearchMatch),
            (3, 5, HighlightKind::SearchMatch),
        ]);
        assert_eq!(m.kind_at(0), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(2), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(3), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(5), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(6), None);
    }

    #[test]
    fn binary_search_multiple_disjoint_ranges() {
        let m = map(&[
            (10, 15, HighlightKind::SearchMatch),
            (20, 25, HighlightKind::SearchMatch),
            (30, 35, HighlightKind::SearchMatch),
        ]);
        assert_eq!(m.kind_at(9),  None);
        assert_eq!(m.kind_at(10), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(15), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(16), None);
        assert_eq!(m.kind_at(20), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(35), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(36), None);
    }

    // ── kind_at: priority / overlap ───────────────────────────────────────────

    #[test]
    fn bracket_wins_over_search_when_overlapping() {
        // Bracket match (pos 12) sits inside a search match (10-15).
        let m = map(&[
            (10, 15, HighlightKind::SearchMatch),
            (12, 12, HighlightKind::BracketMatch),
        ]);
        assert_eq!(m.kind_at(10), Some(HighlightKind::SearchMatch));
        assert_eq!(m.kind_at(12), Some(HighlightKind::BracketMatch)); // bracket wins
        assert_eq!(m.kind_at(13), Some(HighlightKind::SearchMatch));
    }

    #[test]
    fn search_wins_over_nothing() {
        let m = map(&[(5, 10, HighlightKind::SearchMatch)]);
        assert_eq!(m.kind_at(7), Some(HighlightKind::SearchMatch));
    }

    // ── HighlightKind ordering ────────────────────────────────────────────────

    #[test]
    fn bracket_match_has_higher_priority_than_search_match() {
        assert!(HighlightKind::BracketMatch > HighlightKind::SearchMatch);
    }
}
