//! Display-width measurement for UI chrome (popups, pickers, menus, the
//! statusline, the minibuffer) — text with no tab-width context of its own,
//! unlike a buffer line or a decoration's virtual line. Every measurement
//! still funnels through `hume_rope::width`, the workspace's single source
//! of truth, with `display_col` fixed at 0 and `tab_width` at
//! [`hume_rope::width::CHROME_TAB_WIDTH`] — both inert for any non-tab
//! cluster, the only kind this text has.

use hume_engine::types::TruncateEnd;
use hume_rope::width::CHROME_TAB_WIDTH;

/// The truncation marker every chrome truncator (`picker_panel`'s tail
/// clip, `file_path`'s dir/filename shortener) prefixes or appends when it
/// drops text — one glyph, one place its width is asserted, so the two
/// truncators can't silently disagree on how many cells it reserves.
pub const ELLIPSIS: &str = "…";

/// [`ELLIPSIS`]'s display width — always exactly 1: U+2026 HORIZONTAL
/// ELLIPSIS is a single narrow, non-combining Unicode scalar, not a cluster
/// `hume_rope::width` could ever measure differently. A `const` rather than
/// a call to `text_width(ELLIPSIS)` since that measurement isn't itself
/// `const`-evaluable, but the invariant it would report is fixed.
pub const ELLIPSIS_WIDTH: usize = 1;

/// Display width of `s`.
pub fn text_width(s: &str) -> usize {
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
pub fn truncate_text(s: &str, max_display_width: usize) -> (&str, usize) {
    hume_rope::width::truncate_to_width(s, max_display_width, CHROME_TAB_WIDTH)
}

/// Longest suffix of `s` (kept from the end) fitting `max_display_width`
/// display cells, and that suffix's width. Grapheme-cluster aware, never
/// splits a cluster.
pub(crate) fn truncate_text_tail(s: &str, max_display_width: usize) -> (&str, usize) {
    hume_rope::width::truncate_suffix_to_width(s, max_display_width)
}

/// Clip `s` to `budget` display cells per `cut`, marking the dropped end
/// with `…` — list rows (file paths, grep matches, tab labels, …) whose
/// distinguishing part can sit at either end depending on what the caller
/// shows. Grapheme-cluster aware via [`truncate_text`]/[`truncate_text_tail`].
/// Kept distinct from a query row's own tail-truncation (a direct
/// `truncate_text_tail` call) because that text must never gain a marker —
/// user-editable text, whose bare tail (no `…`) is intentional there.
///
/// Borrows `s` unchanged on the (common) no-truncation path instead of
/// allocating a copy of every visible row every frame.
///
/// This module's other helpers are *keep*-oriented (`truncate_text_tail`
/// keeps the tail) while [`TruncateEnd`] is *cut*-oriented, so the arms
/// below read inverted: cutting the head keeps — and thus calls —
/// `truncate_text_tail`.
pub fn truncate_marked(s: &str, budget: usize, cut: TruncateEnd) -> std::borrow::Cow<'_, str> {
    if text_width(s) <= budget {
        return std::borrow::Cow::Borrowed(s);
    }
    if budget == 0 {
        return std::borrow::Cow::Borrowed("");
    }
    let kept = budget.saturating_sub(ELLIPSIS_WIDTH);
    match cut {
        TruncateEnd::Head => {
            let (tail, _) = truncate_text_tail(s, kept);
            std::borrow::Cow::Owned(format!("{ELLIPSIS}{tail}"))
        }
        TruncateEnd::Tail => {
            let (head, _) = truncate_text(s, kept);
            std::borrow::Cow::Owned(format!("{head}{ELLIPSIS}"))
        }
    }
}
