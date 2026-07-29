use std::ops::Range;

use ropey::Rope;
use unicode_width::UnicodeWidthStr;

use crate::pane::{ViewportState, WrapMode};
use crate::providers::GutterColumn;

// ---------------------------------------------------------------------------
// Visible range — output of Stage 1
// ---------------------------------------------------------------------------

/// The output of the Layout stage: which buffer lines are visible and what
/// geometry the Format/Render stages should use.
#[derive(Debug, Clone)]
pub struct VisibleRange {
    /// Buffer lines to format (may extend slightly past the visible area for
    /// smooth-scroll look-ahead; the Render stage clips to `content_height`).
    pub line_range: Range<usize>,
    /// How many display rows to skip from the top of `line_range.start` when
    /// the viewport begins partway through a wrapped line.
    pub top_skip_rows: u16,
    /// Available display rows in the content area.
    pub content_height: u16,
    /// Available columns in the content area (viewport width − gutter width).
    pub content_width: u16,
    /// Total gutter width in columns.
    pub gutter_width: u16,
    /// 0-based index of the last buffer line (`rope.len_lines() - 1`).
    /// This is the correct value to pass to `GutterColumn::width()`.
    pub last_line_idx: usize,
}

// ---------------------------------------------------------------------------
// Stage 1: compute_viewport
// ---------------------------------------------------------------------------

/// Sum of all gutter column widths for the given `max_line` (0-based last line index).
///
/// Pass the last line index of the entire file so gutter width is stable across scrolling.
///
/// Takes an iterator (rather than a slice) so callers can feed it
/// `ProviderSet::gutter_columns()` directly — `ProviderSet` stores
/// `(ProviderId, Box<dyn GutterColumn>)` pairs internally (to support
/// `ProviderSet::remove`), which isn't a shape this purely-arithmetic
/// function needs to know about.
pub fn gutter_width_for_line<'a>(
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    max_line: usize,
) -> u16 {
    gutter_columns.map(|c| c.width(max_line) as u16).sum()
}

/// Compute the `VisibleRange` for a pane given its current state.
///
/// `tab_width` feeds the wrapping-mode line-row estimate (see
/// `estimate_line_rows`) — it has no effect in `WrapMode::None`.
///
/// **Precondition**: `rope` must end with a structural `\n` (the editor's
/// buffer invariant — see project `CLAUDE.md`), except when it is empty.
/// The phantom-trailing-line exclusion below assumes ropey's one extra
/// reported line past `last_line_idx` is that trailing newline's empty
/// remainder; without it, the rope's actual last content line is dropped
/// from `line_range` entirely. Not enforced with a hard error — this
/// crate's own unit tests build ropes without the invariant to test
/// formatting in isolation — but checked with a `debug_assert` so a
/// violation is caught in debug builds instead of silently rendering
/// short.
///
/// This is purely arithmetic — no heap allocations.
pub fn compute_viewport<'a>(
    rope: &Rope,
    viewport: &ViewportState,
    wrap_mode: &WrapMode,
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    tab_width: u8,
) -> VisibleRange {
    debug_assert!(
        rope.len_chars() == 0 || rope.char(rope.len_chars() - 1) == '\n',
        "compute_viewport requires a trailing '\\n' (the buffer invariant) — \
         without it the rope's last content line is dropped from line_range"
    );

    let total_lines = rope.len_lines();
    // 0-based index of the last line — the single source of truth for GutterColumn::width().
    // Using the whole-file last line (not just the visible range) keeps gutter width stable
    // as the user scrolls.
    let last_line_idx = total_lines.saturating_sub(1);
    let gutter_width = gutter_width_for_line(gutter_columns, last_line_idx);

    let content_width = viewport.width.saturating_sub(gutter_width).max(1);

    // Compute buffer line range that fills the viewport.
    let top_line = viewport.top_line.min(last_line_idx.saturating_sub(1));
    let top_skip = viewport.top_row_offset;

    // Exclude the phantom trailing line. The buffer invariant guarantees a
    // trailing `\n`, so ropey always reports one extra empty line at index
    // `last_line_idx`. Real content is lines 0..last_line_idx (exclusive), so
    // `last_line_idx` is the correct exclusive upper bound for the range.
    let line_range = compute_line_range(
        rope,
        top_line,
        top_skip,
        viewport.height,
        content_width,
        wrap_mode,
        last_line_idx,
        tab_width,
    );

    VisibleRange {
        line_range,
        top_skip_rows: top_skip,
        content_height: viewport.height,
        content_width,
        gutter_width,
        last_line_idx,
    }
}

/// Determine which buffer lines need to be formatted to fill `viewport_height`
/// rows, starting from `top_line` with `top_skip` rows already scrolled past.
///
/// `last_line_idx` is the 0-based index of the last real content line (i.e.
/// `rope.len_lines() - 1`). It is used as an exclusive upper bound for the
/// returned range because the phantom trailing line at that index must not be
/// included.
#[allow(clippy::too_many_arguments)]
fn compute_line_range(
    rope: &Rope,
    top_line: usize,
    top_skip: u16,
    viewport_height: u16,
    content_width: u16,
    wrap_mode: &WrapMode,
    last_line_idx: usize,
    tab_width: u8,
) -> Range<usize> {
    // For non-wrapping mode each buffer line is exactly one *content* row —
    // but `top_skip` (`ViewportState::top_row_offset`) can still be nonzero
    // from virtual `before`/`after` rows anchored to a line in range, which
    // this arithmetic doesn't otherwise account for (unlike the wrapping
    // branch below, virtual rows aren't counted per-line here either). Add
    // it into the budget the same generous-by-construction way: it can only
    // over-supply lines (the render stage clips extras via `vc.is_full()`),
    // never under-supply the bottom of the range.
    if !wrap_mode.is_wrapping() {
        let end = (top_line + viewport_height as usize + top_skip as usize).min(last_line_idx);
        return top_line..end;
    }

    // For wrapping modes: count rows per line until we have filled the viewport.
    // `top_skip` rows have been consumed from `top_line` already.
    let mut rows_needed = viewport_height as usize + top_skip as usize;
    let mut end = top_line;

    while end < last_line_idx && rows_needed > 0 {
        let line_rows = estimate_line_rows(rope, end, content_width, tab_width);
        rows_needed = rows_needed.saturating_sub(line_rows);
        end += 1;
    }

    // Add a small look-ahead so smooth scrolling has room. Word/Indent wrap
    // can still exceed this estimate (a short word wastes columns) — that
    // residual error is what the look-ahead buffers against. Exact counting
    // would mean running the formatter per line, rejected: this stage is
    // documented as purely arithmetic, no allocations.
    const LOOKAHEAD_LINES: usize = 4;
    let end = (end + LOOKAHEAD_LINES).min(last_line_idx);
    top_line..end
}

/// Cheaply estimate how many display rows a buffer line occupies when wrapped
/// to `content_width`.
///
/// Width-aware: sums `unicode_width` display width over the line's chunks,
/// swapping each tab's 1-column placeholder width for a full `tab_width`
/// expansion — a deliberate upper bound, since it ignores the tab's actual
/// column position, so it can only over-count, never under-count (safe for a
/// look-ahead bound). Character count alone (the previous proxy) undercounts
/// CJK (2 columns) and tabs (up to `tab_width` columns); overestimates here
/// are harmless — the extra lines get formatted, then clipped by the Render
/// stage.
fn estimate_line_rows(rope: &Rope, line_idx: usize, content_width: u16, tab_width: u8) -> usize {
    if content_width == 0 {
        return 1;
    }
    let tab_width = tab_width.max(1) as usize;
    let line = rope.line(line_idx);
    let mut total_width = 0usize;
    for chunk in line.chunks() {
        let tab_count = chunk.matches('\t').count();
        // `unicode_width` reports width 1 (not 0) for '\t' in this crate
        // version — subtract that placeholder count back out before adding
        // the real tab-stop expansion, so each tab contributes exactly
        // `tab_width` columns to the estimate (a deliberate upper bound: it
        // ignores the tab's actual column position, so it can only
        // over-count, never under-count).
        total_width += chunk.width() - tab_count + tab_count * tab_width;
    }
    if total_width == 0 {
        1
    } else {
        total_width.div_ceil(content_width as usize).max(1)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
