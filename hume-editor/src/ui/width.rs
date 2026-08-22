//! Display-width measurement for UI chrome (popups, pickers, menus, the
//! statusline, the minibuffer) — text with no tab-width context of its own,
//! unlike a buffer line or a decoration's virtual row. Every measurement
//! still funnels through `hume_rope::width`, the workspace's single source
//! of truth, with `col`/`tab_width` fixed at values that are inert for any
//! non-tab cluster — the only kind this text has.

/// Display width of `s`.
pub(crate) fn text_width(s: &str) -> usize {
    hume_rope::width::str_width(s, 0, 1)
}

/// Display width of one grapheme cluster.
pub(crate) fn cell_width(g: &str) -> usize {
    hume_rope::width::grapheme_width(g, 0, 1)
}
