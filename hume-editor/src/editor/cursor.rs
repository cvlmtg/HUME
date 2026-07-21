//! Terminal cursor placement logic.
//!
//! The terminal cursor (the blinking bar or block emitted via escape sequences)
//! is an editor-level concern. The engine knows nothing about it — it only
//! styles the grapheme at each selection head.
//!
//! This module computes:
//! - [`screen_pos`] — the `(col, row)` of the primary selection head in the
//!   pane content area (terminal cursor placement).
//! - [`gutter_width`] — the gutter offset to add so the terminal cursor lands
//!   at the correct absolute screen column.
//! - [`sub_row`] — which wrapped display row the primary selection head is on
//!   (used by scroll to keep the head visible).

use hume_engine::format::{FormatScratch, display_rows_for_line};
use hume_engine::layout::gutter_width_for_line;
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use hume_engine::pipeline::RenderContext;
use hume_engine::providers::{GutterColumn, ProviderSet};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the on-screen `(col, row)` of `cursor_char` within the pane content
/// area (i.e., after the gutter).
///
/// Returns `None` if the position is outside the visible viewport (defensive;
/// should not happen after `scroll::ensure_cursor_visible`).
///
/// In no-wrap mode, `col` accounts for `viewport.horizontal_offset`.
/// In wrap mode, `col` is the column within the display row (offset 0 = left edge).
#[allow(clippy::too_many_arguments)]
pub(crate) fn screen_pos(
    viewport: &ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    ctx: &mut RenderContext,
    providers: &ProviderSet,
    content_width: u16,
) -> Option<(u16, u16)> {
    let scratch = &mut ctx.cursor_format;
    let cursor_line = rope.char_to_line(cursor_char);
    let height = viewport.height as usize;
    if height == 0 {
        return None;
    }

    let (cursor_sub, cursor_col) = format_row_col(
        rope,
        cursor_line,
        cursor_char,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
    );

    if wrap_mode.is_wrapping() {
        let top_row = viewport.top_row_offset as usize;
        let mut screen_row = 0usize;

        for line_idx in viewport.top_line..=cursor_line {
            let is_top = line_idx == viewport.top_line;
            let skip = if is_top { top_row } else { 0 };
            let breakdown = display_rows_for_line(
                rope,
                line_idx,
                tab_width,
                whitespace,
                wrap_mode,
                providers,
                content_width,
                scratch,
            );
            // `top_row_offset` only ever indexes into a line's *content* rows
            // (virtual rows never partially scroll — see `ViewportState`'s
            // own doc) — so the viewport's own top line never shows its
            // `before` block: it's either fully above the viewport already
            // (scrolled past) or the natural at-rest state simply starts
            // this line at its content, not its virtual annotation.
            let visible_before = if is_top { 0 } else { breakdown.before };
            if line_idx == cursor_line {
                screen_row += visible_before + cursor_sub.saturating_sub(skip);
                break;
            }
            screen_row +=
                visible_before + (breakdown.content + breakdown.after).saturating_sub(skip);
            if screen_row >= height {
                return None;
            }
        }

        if screen_row >= height {
            return None;
        }
        Some((cursor_col as u16, screen_row as u16))
    } else {
        if cursor_line < viewport.top_line {
            return None;
        }
        let screen_row = cursor_line - viewport.top_line;
        if screen_row >= height {
            return None;
        }

        let col = cursor_col.saturating_sub(viewport.horizontal_offset as usize);
        Some((col as u16, screen_row as u16))
    }
}

/// Gutter width in terminal columns for the current frame.
///
/// Used to offset the terminal cursor column past line numbers and other gutter
/// providers.
pub(crate) fn gutter_width<'a>(
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    total_lines: usize,
) -> u16 {
    gutter_width_for_line(gutter_columns, total_lines.saturating_sub(1))
}

/// Which wrapped display sub-row of buffer `line_idx` contains `cursor_char`.
///
/// Used by `scroll::ensure_cursor_visible` to keep the selection head visible.
pub(crate) fn sub_row(
    rope: &ropey::Rope,
    line_idx: usize,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) -> usize {
    format_row_col(
        rope,
        line_idx,
        cursor_char,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
    )
    .0
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Format `line_idx` and locate `cursor_char` within the resulting display rows.
///
/// Returns `(sub_row, col)` where `sub_row` is the 0-based display row index
/// within the line, and `col` is the display column within that row (the
/// grapheme's `col` field from the engine format output).
pub(crate) fn format_row_col(
    rope: &ropey::Rope,
    line_idx: usize,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) -> (usize, usize) {
    let line_start_char = rope.line_to_char(line_idx);
    let line_start_byte = rope.char_to_byte(line_start_char);
    let cursor_byte_abs = rope.char_to_byte(cursor_char);
    let cursor_byte_in_line = cursor_byte_abs.saturating_sub(line_start_byte);

    scratch.display_rows.clear();
    scratch.graphemes.clear();
    scratch.line_texts.clear();
    hume_engine::format::format_buffer_line(
        rope,
        line_idx,
        tab_width,
        whitespace,
        wrap_mode,
        None,
        &[],
        scratch,
    );

    for (i, row) in scratch.display_rows.iter().enumerate() {
        if row.graphemes.is_empty() {
            continue;
        }
        let first = &scratch.graphemes[row.graphemes.start];
        let last = &scratch.graphemes[row.graphemes.end - 1];
        let row_byte_start = first.byte_range.start;
        let row_byte_end = last.byte_range.end;
        let is_last = i + 1 == scratch.display_rows.len();

        if cursor_byte_in_line >= row_byte_start && (cursor_byte_in_line < row_byte_end || is_last)
        {
            let col = scratch.graphemes[row.graphemes.clone()]
                .iter()
                .find(|g| g.byte_range.start == cursor_byte_in_line)
                .map_or_else(
                    || {
                        // Selection head is past all graphemes in this row (e.g., at eol).
                        let lg = &scratch.graphemes[row.graphemes.end - 1];
                        (lg.col + lg.width as u16) as usize
                    },
                    |g| g.col as usize,
                );
            return (i, col);
        }
    }

    // Fallback: last sub-row, column past last grapheme.
    let last = scratch.display_rows.len().saturating_sub(1);
    let col = scratch
        .display_rows
        .get(last)
        .filter(|r| !r.graphemes.is_empty())
        .map(|r| {
            let lg = &scratch.graphemes[r.graphemes.end - 1];
            (lg.col + lg.width as u16) as usize
        })
        .unwrap_or(0);
    (last, col)
}

// ---------------------------------------------------------------------------
// Screen-to-buffer reverse mapping
// ---------------------------------------------------------------------------

/// Convert a terminal-absolute `(screen_x, screen_y)` click position to a
/// buffer char offset.
///
/// `gutter_w` is the width of the gutter in terminal columns (from
/// [`gutter_width`]). Clicks that land inside the gutter return `None`.
///
/// The coordinate space is pane-relative: `(0, 0)` is the top-left cell of
/// the pane, matching what `MouseEvent.column` / `.row` report
/// when the pane fills the whole terminal (which is currently always true).
///
/// Returns `None` if the click is:
/// - in the gutter,
/// - below the last buffer line, or
/// - the buffer is empty.
#[allow(clippy::too_many_arguments)]
pub(crate) fn screen_to_char_offset(
    screen_x: u16,
    screen_y: u16,
    gutter_w: u16,
    viewport: &ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
    providers: &ProviderSet,
    content_width: u16,
) -> Option<usize> {
    // Clicks inside the gutter (line numbers etc.) do not map to text.
    if screen_x < gutter_w {
        return None;
    }

    let total_lines = rope.len_lines();
    // A buffer always ends with '\n', so the last "line" in ropey is an empty
    // sentinel. The real last editable line is `total_lines - 2` (or 0 for a
    // one-line buffer that is just "\n").
    let last_real_line = total_lines.saturating_sub(2);

    let target_row = screen_y as usize;

    if wrap_mode.is_wrapping() {
        // Walk from the top of the viewport counting display rows until we
        // reach the target screen row.
        let mut remaining = target_row;
        let top_row = viewport.top_row_offset as usize;

        for line_idx in viewport.top_line..total_lines {
            let is_top = line_idx == viewport.top_line;
            let skip = if is_top { top_row } else { 0 };
            let breakdown = display_rows_for_line(
                rope,
                line_idx,
                tab_width,
                whitespace,
                wrap_mode,
                providers,
                content_width,
                scratch,
            );
            // Same "the viewport's own top line never shows its `before`
            // block" reasoning as `screen_pos`.
            let visible_before = if is_top { 0 } else { breakdown.before };
            let content_visible = breakdown.content.saturating_sub(skip);
            let visible_rows = visible_before + content_visible + breakdown.after;

            if remaining < visible_rows {
                // A click landing in the `before`/`after` portion is on a
                // virtual row, not buffer content — clamp to this line's
                // first/last content sub-row. Precisely mapping such a click
                // to its anchor line's exact position needs a
                // real `VirtualLineSource` to have anything to map from;
                // this only needs to degrade sensibly with zero providers
                // registered, which is always true here.
                let target_sub = if remaining < visible_before {
                    0
                } else if remaining < visible_before + content_visible {
                    skip + (remaining - visible_before)
                } else {
                    breakdown.content.saturating_sub(1)
                };
                return char_at_display_col(
                    screen_x - gutter_w,
                    target_sub,
                    line_idx,
                    rope,
                    tab_width,
                    whitespace,
                    wrap_mode,
                    scratch,
                );
            }

            remaining = remaining.saturating_sub(visible_rows);
            if line_idx >= last_real_line {
                break;
            }
        }
        // Click is below the last line — clamp to end of last real line.
        char_at_display_col(
            screen_x - gutter_w,
            // sub-row doesn't matter much; last sub will be used anyway
            usize::MAX,
            last_real_line,
            rope,
            tab_width,
            whitespace,
            wrap_mode,
            scratch,
        )
    } else {
        // No-wrap: each buffer line is exactly one display row.
        let line_idx = (viewport.top_line + target_row).min(last_real_line);

        // Content column = screen column past gutter + horizontal scroll offset.
        let content_col = (screen_x - gutter_w) as usize + viewport.horizontal_offset as usize;

        // Format the line and find the grapheme at `content_col`.
        scratch.display_rows.clear();
        scratch.graphemes.clear();
        scratch.line_texts.clear();
        hume_engine::format::format_buffer_line(
            rope,
            line_idx,
            tab_width,
            whitespace,
            wrap_mode,
            None,
            &[],
            scratch,
        );

        if scratch.display_rows.is_empty() {
            return Some(rope.line_to_char(line_idx));
        }
        let row = &scratch.display_rows[0];
        Some(col_to_char_offset(
            content_col,
            row,
            scratch,
            rope,
            line_idx,
        ))
    }
}

/// Given a target display column and a `DisplayRow`, return the char offset of
/// the grapheme that best matches (or the last grapheme if past the end).
fn col_to_char_offset(
    target_col: usize,
    row: &hume_engine::types::DisplayRow,
    scratch: &hume_engine::format::FormatScratch,
    rope: &ropey::Rope,
    line_idx: usize,
) -> usize {
    let graphemes = &scratch.graphemes[row.graphemes.clone()];
    if graphemes.is_empty() {
        return rope.line_to_char(line_idx);
    }

    // Find the grapheme whose column range contains `target_col`.
    for g in graphemes {
        let g_end = g.col as usize + g.width as usize;
        if target_col < g_end {
            return g.char_offset;
        }
    }
    // Past the last grapheme — return the last char offset in the row.
    graphemes
        .last()
        .map(|g| g.char_offset)
        .unwrap_or_else(|| rope.line_to_char(line_idx))
}

/// Find the char offset for `(content_col, target_sub_row)` within
/// `line_idx`, using the engine format pipeline.
#[allow(clippy::too_many_arguments)]
fn char_at_display_col(
    content_col: u16,
    target_sub: usize,
    line_idx: usize,
    rope: &ropey::Rope,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    scratch: &mut FormatScratch,
) -> Option<usize> {
    scratch.display_rows.clear();
    scratch.graphemes.clear();
    scratch.line_texts.clear();
    hume_engine::format::format_buffer_line(
        rope,
        line_idx,
        tab_width,
        whitespace,
        wrap_mode,
        None,
        &[],
        scratch,
    );

    if scratch.display_rows.is_empty() {
        return Some(rope.line_to_char(line_idx));
    }

    // Clamp target sub-row to the last display row of this line.
    let sub = target_sub.min(scratch.display_rows.len().saturating_sub(1));
    let row = &scratch.display_rows[sub];
    Some(col_to_char_offset(
        content_col as usize,
        row,
        scratch,
        rope,
        line_idx,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
