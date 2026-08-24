//! Display-width measurement for UI chrome (popups, pickers, menus, the
//! statusline, the minibuffer) — text with no tab-width context of its own,
//! unlike a buffer line or a decoration's virtual row. Every measurement
//! still funnels through `hume_rope::width`, the workspace's single source
//! of truth, with `display_col` fixed at 0 and `tab_width` at
//! [`hume_rope::width::CHROME_TAB_WIDTH`] — both inert for any non-tab
//! cluster, the only kind this text has.

use hume_rope::width::CHROME_TAB_WIDTH;

/// The truncation marker every chrome truncator (`picker_panel`'s tail
/// clip, `file_path`'s dir/filename shortener) prefixes or appends when it
/// drops text — one glyph, one place its width is asserted, so the two
/// truncators can't silently disagree on how many cells it reserves.
pub(crate) const ELLIPSIS: &str = "…";

/// [`ELLIPSIS`]'s display width — always exactly 1: U+2026 HORIZONTAL
/// ELLIPSIS is a single narrow, non-combining Unicode scalar, not a cluster
/// `hume_rope::width` could ever measure differently. A `const` rather than
/// a call to `text_width(ELLIPSIS)` since that measurement isn't itself
/// `const`-evaluable, but the invariant it would report is fixed.
pub(crate) const ELLIPSIS_WIDTH: usize = 1;

/// Display width of `s`.
pub(crate) fn text_width(s: &str) -> usize {
    hume_rope::width::str_width(s, 0, CHROME_TAB_WIDTH)
}

/// Display width of one grapheme cluster.
pub(crate) fn cell_width(g: &str) -> usize {
    hume_rope::width::grapheme_width(g, 0, CHROME_TAB_WIDTH)
}

/// Longest prefix of `s` fitting `max_display_width` display cells, and
/// that prefix's width. Grapheme-cluster aware, never splits a cluster.
/// Returns the width alongside the text so a caller that needs both (to
/// place whatever comes after) doesn't re-measure what this already
/// computed.
pub(crate) fn truncate_text(s: &str, max_display_width: usize) -> (&str, usize) {
    hume_rope::width::truncate_to_width(s, max_display_width, CHROME_TAB_WIDTH)
}

/// Longest suffix of `s` (kept from the end) fitting `max_display_width`
/// display cells, and that suffix's width. Grapheme-cluster aware, never
/// splits a cluster.
pub(crate) fn truncate_text_tail(s: &str, max_display_width: usize) -> (&str, usize) {
    hume_rope::width::truncate_suffix_to_width(s, max_display_width)
}
