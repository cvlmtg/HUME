//! Scroll logic for the engine-based viewport.
//!
//! Operates on `hume_engine::pane::ViewportState` and `ropey::Rope`.
//! Called from `Editor::run()` via `scroll::ensure_cursor_visible(...)`.

use hume_engine::format::{FormatScratch, display_rows_for_line};
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use hume_engine::providers::ProviderSet;

use super::cursor;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Adjust `viewport.top_line`/`top_row_offset` so the cursor's display row
/// is visible with `v_margin` rows of look-ahead.
///
/// `providers`/`content_width` feed the virtual-row-aware row counting
/// (`display_rows_for_line`). Wrap-mode-agnostic: `display_rows_for_line`
/// returns `content: 1` for `WrapMode::None`, so the same block-row
/// accounting (`before`/content/`after`, per `ViewportState::top_row_offset`'s
/// doc) applies whether or not the line itself wraps.
pub(super) fn ensure_cursor_visible(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
    v_margin: usize,
    providers: &ProviderSet,
    content_width: u16,
) {
    let cursor_line = rope.char_to_line(cursor_char);
    let height = viewport.height as usize;
    if height == 0 {
        return;
    }

    let margin = v_margin.min(height / 2);

    let cursor_sub = cursor::sub_row(
        rope,
        cursor_line,
        cursor_char,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
    );
    // Row of the cursor within its own line's visual block (`before` +
    // content + `after` — see `ViewportState::top_row_offset`'s doc).
    let cursor_breakdown = display_rows_for_line(
        rope,
        cursor_line,
        tab_width,
        whitespace,
        wrap_mode,
        providers,
        content_width,
        scratch,
    );
    let cursor_block_row = cursor_breakdown.before + cursor_sub;

    // ── Cursor above the viewport ────────────────────────────────────────────
    let top_row = viewport.top_row_offset as usize;
    if cursor_line < viewport.top_line
        || (cursor_line == viewport.top_line && cursor_block_row < top_row)
    {
        scroll_backward_from_cursor(
            viewport,
            rope,
            cursor_line,
            cursor_sub,
            margin,
            wrap_mode,
            tab_width,
            whitespace,
            scratch,
            providers,
            content_width,
        );
        return; // cursor above viewport — done
    }

    // ── Count display rows from scroll position to cursor ────────────────────
    // Same before/content/after accumulation as `cursor::screen_pos`.
    let mut display_row: usize = 0;
    for line_idx in viewport.top_line..=cursor_line {
        let is_top = line_idx == viewport.top_line;
        let skip = if is_top { top_row } else { 0 };
        if line_idx == cursor_line {
            display_row += cursor_block_row.saturating_sub(skip);
            break;
        }
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
        display_row += breakdown.total().saturating_sub(skip);
        if display_row >= height {
            break;
        }
    }

    // ── Cursor too close to the top edge ────────────────────────────────────
    if display_row < margin {
        scroll_backward_from_cursor(
            viewport,
            rope,
            cursor_line,
            cursor_sub,
            margin,
            wrap_mode,
            tab_width,
            whitespace,
            scratch,
            providers,
            content_width,
        );
        return;
    }

    // ── Cursor too close to the bottom edge ──────────────────────────────────
    if display_row >= height.saturating_sub(margin) {
        let target_row = height.saturating_sub(margin).saturating_sub(1);
        scroll_backward_from_cursor(
            viewport,
            rope,
            cursor_line,
            cursor_sub,
            target_row,
            wrap_mode,
            tab_width,
            whitespace,
            scratch,
            providers,
            content_width,
        );
    }
}

/// Clamp `viewport.top_row_offset` to a valid row of `top_line`'s current
/// visual block.
///
/// Single self-heal chokepoint for staleness: nothing else in the codebase
/// validates a `top_row_offset` write against the block it actually refers
/// to (`Pane::recall_scroll` restores a saved offset verbatim; an LSP
/// goto-definition jump moves `top_line` without touching `top_row_offset`
/// at all) — the block a stale offset was valid for can shrink or disappear
/// entirely (wrap width change, a `VirtualLineSource` removed, a resize)
/// between the write and the next read. Call once per pane per frame, before
/// `ensure_cursor_visible`, so every other write site can stay unvalidated.
#[allow(clippy::too_many_arguments)]
pub(super) fn clamp_top_row_offset(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
    providers: &ProviderSet,
    content_width: u16,
) {
    let total = display_rows_for_line(
        rope,
        viewport.top_line,
        tab_width,
        whitespace,
        wrap_mode,
        providers,
        content_width,
        scratch,
    )
    .total();
    viewport.top_row_offset = viewport.top_row_offset.min(total.saturating_sub(1) as u16);
}

/// Adjust `viewport.horizontal_offset` so the cursor's display column stays
/// visible. When wrapping is active, horizontal offset is forced to 0
/// (wrapping handles long lines). The horizontal margin is fixed —
/// `scrolloff` only governs the vertical axis.
pub(super) fn ensure_cursor_visible_horizontal(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) {
    const H_MARGIN: usize = 5;

    if wrap_mode.is_wrapping() {
        viewport.horizontal_offset = 0;
        return;
    }

    let cursor_line = rope.char_to_line(cursor_char);
    let (_sub_row, cursor_col) = cursor::format_row_col(
        rope,
        cursor_line,
        cursor_char,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
    );
    let content_width = viewport.width as usize;
    if content_width == 0 {
        return;
    }

    let margin = H_MARGIN.min(content_width / 2);
    let offset = viewport.horizontal_offset as usize;

    if cursor_col < offset + margin {
        viewport.horizontal_offset = cursor_col.saturating_sub(margin) as u16;
    } else if cursor_col >= offset + content_width - margin {
        viewport.horizontal_offset = cursor_col.saturating_sub(content_width - margin - 1) as u16;
    }
}

#[allow(clippy::too_many_arguments)]
/// Scroll the viewport so the cursor's display row lands at `target_row`
/// (0-based) inside the visible area. Used by `zz`/`zt`/`zb`-style commands.
///
/// Top-of-buffer is clamped to `top_line == 0`; bottom-of-buffer is *not*
/// clamped (vim/Helix semantics — empty rows past EOF are allowed).
pub(super) fn scroll_cursor_to_row(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
    target_row: usize,
    providers: &ProviderSet,
    content_width: u16,
) {
    let cursor_line = rope.char_to_line(cursor_char);
    let cursor_sub = cursor::sub_row(
        rope,
        cursor_line,
        cursor_char,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
    );
    scroll_backward_from_cursor(
        viewport,
        rope,
        cursor_line,
        cursor_sub,
        target_row,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
        providers,
        content_width,
    );
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn scroll_backward_from_cursor(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_line: usize,
    cursor_sub: usize,
    target_rows: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
    providers: &ProviderSet,
    content_width: u16,
) {
    viewport.top_line = cursor_line;
    // Seed at the cursor's row within its own line's visual block (`before`
    // + content + `after`), then walk backward one block row at a time —
    // every row is an equal scroll unit, so this can land on any row of any
    // block, including mid-`before`/`after`.
    let cursor_breakdown = display_rows_for_line(
        rope,
        cursor_line,
        tab_width,
        whitespace,
        wrap_mode,
        providers,
        content_width,
        scratch,
    );
    viewport.top_row_offset = (cursor_breakdown.before + cursor_sub) as u16;
    let mut rows_above = 0;
    while rows_above < target_rows {
        if viewport.top_row_offset > 0 {
            viewport.top_row_offset -= 1;
            rows_above += 1;
        } else if viewport.top_line > 0 {
            viewport.top_line -= 1;
            let breakdown = display_rows_for_line(
                rope,
                viewport.top_line,
                tab_width,
                whitespace,
                wrap_mode,
                providers,
                content_width,
                scratch,
            );
            let total = breakdown.total();
            if rows_above + total > target_rows {
                // Uniform rows mean the cut point lands exactly here, no
                // clamping needed — this line contributes exactly
                // `remaining` visible rows above the cursor.
                let remaining = target_rows - rows_above;
                viewport.top_row_offset = (total - remaining) as u16;
                break;
            }
            rows_above += total;
        } else {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
