//! Display-width measurement for UI chrome (popups, pickers, menus, the
//! statusline, the minibuffer) — text with no tab-width context of its own,
//! unlike a buffer line or a decoration's virtual row. Every measurement
//! still funnels through `hume_rope::width`, the workspace's single source
//! of truth, with `col` fixed at 0 and `tab_width` at
//! [`hume_rope::width::CHROME_TAB_WIDTH`] — both inert for any non-tab
//! cluster, the only kind this text has.

use hume_rope::width::CHROME_TAB_WIDTH;

/// Display width of `s`.
pub(crate) fn text_width(s: &str) -> usize {
    hume_rope::width::str_width(s, 0, CHROME_TAB_WIDTH)
}

/// Display width of one grapheme cluster.
pub(crate) fn cell_width(g: &str) -> usize {
    hume_rope::width::grapheme_width(g, 0, CHROME_TAB_WIDTH)
}

/// Longest prefix of `s` fitting `max_display_width` display cells.
/// Grapheme-cluster aware, never splits a cluster.
pub(crate) fn truncate_text(s: &str, max_display_width: usize) -> &str {
    hume_rope::width::truncate_to_width(s, max_display_width, CHROME_TAB_WIDTH).0
}

/// Longest suffix of `s` (kept from the end) fitting `max_display_width`
/// display cells. Grapheme-cluster aware, never splits a cluster.
pub(crate) fn truncate_text_tail(s: &str, max_display_width: usize) -> &str {
    hume_rope::width::truncate_suffix_to_width(s, max_display_width, CHROME_TAB_WIDTH).0
}
