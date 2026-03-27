use ratatui::style::Style;

/// A set of ephemeral highlights to overlay on the document.
///
/// Each entry is an inclusive char range `[start, end]` plus a `Style`.
/// Build once per frame (after all `push` calls, call `build` to sort),
/// then query per character with `style_at`.
///
/// Used for bracket matching now; search hits and diagnostics later.
pub(crate) struct HighlightSet {
    /// Sorted by `start` after `build()` is called. Non-overlapping.
    entries: Vec<(usize, usize, Style)>,
}

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
    pub(crate) fn style_at(&self, pos: usize) -> Option<Style> {
        // partition_point returns the first index where the predicate is false,
        // i.e. the first entry with start > pos.
        let idx = self.entries.partition_point(|&(s, _, _)| s <= pos);
        let (_, end, style) = *self.entries.get(idx.checked_sub(1)?)?;
        (pos <= end).then_some(style)
    }
}
