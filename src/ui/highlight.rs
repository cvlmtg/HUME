use ratatui::style::Style;

/// A set of ephemeral highlights to overlay on the document.
///
/// Each entry is an inclusive char range `[start, end]` plus a `Style`.
/// Build once per frame (after all `push` calls, call `build` to sort),
/// then query per character with `style_at`.
///
/// Used for bracket matching now; search hits and diagnostics later.
pub(crate) struct HighlightSet {
    /// Sorted by `start` after `build()` is called.
    entries: Vec<(usize, usize, Style)>,
}

/// A permanent empty highlight set. Use `&EMPTY` instead of
/// `&HighlightSet::new()` when no highlights are needed — it avoids a heap
/// allocation and can be stored as a `&'static HighlightSet`.
pub(crate) static EMPTY: HighlightSet = HighlightSet { entries: Vec::new() };

impl HighlightSet {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub(crate) fn push(&mut self, start: usize, end: usize, style: Style) {
        self.entries.push((start, end, style));
    }

    /// Sort entries by start position so `style_at` can binary-search.
    /// Call once after all `push` calls, before querying.
    pub(crate) fn build(mut self) -> Self {
        self.entries.sort_unstable_by_key(|&(s, _, _)| s);
        self
    }

    /// Return the style for `pos` if it falls within any highlight range.
    ///
    /// Requires `build()` to have been called. O(log n) via binary search on
    /// the sorted entry list: find the last range whose start ≤ pos, then
    /// check if pos ≤ end.
    ///
    /// Currently only bracket-match highlights exist so at most one range
    /// overlaps any position. When search/diagnostic highlights are added,
    /// this must be extended to scan all overlapping entries and compose styles.
    pub(crate) fn style_at(&self, pos: usize) -> Option<Style> {
        // partition_point returns the first index where the predicate is false,
        // i.e. the first entry with start > pos.
        let idx = self.entries.partition_point(|&(s, _, _)| s <= pos);
        let (_, end, style) = *self.entries.get(idx.checked_sub(1)?)?;
        (pos <= end).then_some(style)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};

    use super::*;

    fn red() -> Style { Style::new().fg(Color::Red) }
    fn blue() -> Style { Style::new().fg(Color::Blue) }
    fn green() -> Style { Style::new().fg(Color::Green) }

    fn set(ranges: &[(usize, usize, Style)]) -> HighlightSet {
        let mut hl = HighlightSet::new();
        for &(s, e, style) in ranges {
            hl.push(s, e, style);
        }
        hl.build()
    }

    #[test]
    fn empty_returns_none() {
        let hl = &EMPTY;
        assert_eq!(hl.style_at(0), None);
        assert_eq!(hl.style_at(100), None);
    }

    #[test]
    fn single_point_range() {
        // push(5, 5, ..) matches only pos 5, not 4 or 6.
        let hl = set(&[(5, 5, red())]);
        assert_eq!(hl.style_at(4), None);
        assert_eq!(hl.style_at(5), Some(red()));
        assert_eq!(hl.style_at(6), None);
    }

    #[test]
    fn pos_before_all_ranges() {
        // Query at 0 when the first range starts at 5 — must not wrap around.
        let hl = set(&[(5, 10, red())]);
        assert_eq!(hl.style_at(0), None);
        assert_eq!(hl.style_at(4), None);
        assert_eq!(hl.style_at(5), Some(red()));
    }

    #[test]
    fn adjacent_ranges_do_not_bleed() {
        // [0,2] and [3,5] share no positions — pos 2 and 3 belong to separate ranges.
        let hl = set(&[(0, 2, red()), (3, 5, blue())]);
        assert_eq!(hl.style_at(0), Some(red()));
        assert_eq!(hl.style_at(2), Some(red()));
        assert_eq!(hl.style_at(3), Some(blue()));
        assert_eq!(hl.style_at(5), Some(blue()));
        assert_eq!(hl.style_at(6), None);
    }

    #[test]
    fn binary_search_multiple_ranges() {
        // Three disjoint ranges — verify correct style at each and None in gaps.
        let hl = set(&[(10, 15, red()), (20, 25, blue()), (30, 35, green())]);
        assert_eq!(hl.style_at(9),  None);
        assert_eq!(hl.style_at(10), Some(red()));
        assert_eq!(hl.style_at(15), Some(red()));
        assert_eq!(hl.style_at(16), None);
        assert_eq!(hl.style_at(20), Some(blue()));
        assert_eq!(hl.style_at(25), Some(blue()));
        assert_eq!(hl.style_at(26), None);
        assert_eq!(hl.style_at(30), Some(green()));
        assert_eq!(hl.style_at(35), Some(green()));
        assert_eq!(hl.style_at(36), None);
    }
}
