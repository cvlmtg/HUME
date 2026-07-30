//! Scroll logic for the engine-based viewport.
//!
//! Every routine here is a thin consumer of `hume_engine::rows::RowMap`, the
//! one authority on the document's display-row list. Scrolling is then just
//! arithmetic on row addresses: "put the cursor's row `n` rows below the top"
//! is `advance(cursor, -n)`, in either wrap mode, with or without virtual rows.

use hume_engine::pane::ViewportState;
use hume_engine::rows::{RowMap, RowPos};

// ---------------------------------------------------------------------------
// Viewport ↔ row address
// ---------------------------------------------------------------------------

/// The viewport's top display row as a row address.
///
/// `ViewportState`'s `top_line`/`top_row_offset` pair *is* a `RowPos` — this
/// and [`set_top`] are the only two places that spelling is converted, so no
/// caller re-derives what `top_row_offset` counts.
pub(super) fn top_pos(viewport: &ViewportState) -> RowPos {
    RowPos::new(viewport.top_line, viewport.top_row_offset as usize)
}

/// Move the viewport's top to `pos`.
pub(super) fn set_top(viewport: &mut ViewportState, pos: RowPos) {
    viewport.top_line = pos.line;
    viewport.top_row_offset = u16::try_from(pos.row).unwrap_or(u16::MAX);
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Adjust the viewport so the cursor's display row is visible with `v_margin`
/// rows of look-ahead above and below it.
pub(super) fn ensure_cursor_visible(
    viewport: &mut ViewportState,
    rm: &mut RowMap<'_>,
    cursor_char: usize,
    v_margin: usize,
) {
    let height = viewport.height as usize;
    if height == 0 {
        return;
    }
    // `(height - 1) / 2`, not `height / 2`: at an even height, a margin of
    // exactly `height / 2` leaves arm 2's stable window empty (its bounds
    // `margin..height-margin` collapse to a single point), so the two
    // correction arms fight over that one row and rescroll every frame.
    let margin = v_margin.min(height.saturating_sub(1) / 2);
    let (cursor_pos, _) = rm.locate(cursor_char);
    let top = top_pos(viewport);

    if cursor_pos < top {
        scroll_back_from(viewport, rm, cursor_pos, margin);
        return;
    }

    // `None` means the cursor is more than a viewport's worth of rows below
    // the top, which wants the same correction as any other too-low cursor.
    match rm.distance(top, cursor_pos, height) {
        Some(rows_down) if rows_down < margin => {
            scroll_back_from(viewport, rm, cursor_pos, margin);
        }
        Some(rows_down) if rows_down < height.saturating_sub(margin) => {}
        _ => {
            let target = height.saturating_sub(margin).saturating_sub(1);
            scroll_back_from(viewport, rm, cursor_pos, target);
        }
    }
}

/// Pull the viewport's top onto a row that actually exists.
///
/// Single self-heal chokepoint for staleness: nothing else in the codebase
/// validates a `top_line`/`top_row_offset` write against the block it refers
/// to (`Pane::recall_scroll` restores a saved offset verbatim; an LSP
/// goto-definition jump moves `top_line` without touching `top_row_offset` at
/// all) — and the block a stale address was valid for can shrink or vanish
/// (wrap width change, a `VirtualLineSource` removed, a resize) between the
/// write and the next read. Call once per pane per frame, before
/// `ensure_cursor_visible`, so every other write site can stay unvalidated.
pub(super) fn clamp_viewport_top(viewport: &mut ViewportState, rm: &mut RowMap<'_>) {
    let clamped = rm.clamp(top_pos(viewport));
    set_top(viewport, clamped);
}

/// Adjust `viewport.horizontal_offset` so the cursor's display column stays
/// visible. Wrapping modes have no horizontal scroll, so the offset is forced
/// to 0 there. The horizontal margin is fixed — `scrolloff` governs only the
/// vertical axis.
pub(super) fn ensure_cursor_visible_horizontal(
    viewport: &mut ViewportState,
    rm: &mut RowMap<'_>,
    cursor_char: usize,
) {
    const H_MARGIN: usize = 5;

    if rm.is_wrapping() {
        viewport.horizontal_offset = 0;
        return;
    }

    let (_, cursor_col) = rm.locate(cursor_char);
    let cursor_col = cursor_col as usize;
    // `locate`'s column is content-relative (the gutter isn't part of it),
    // so the margin must compare against the content width the map itself
    // was built with — not `viewport.width`, which still includes the
    // gutter and so under-counts how many columns are actually visible.
    let content_width = rm.content_width() as usize;
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

/// Scroll so the cursor's display row lands `target_row` rows below the top of
/// the viewport. Used by `zz`/`zt`/`zb`-style commands.
///
/// Top-of-buffer is clamped to the document's first row; bottom-of-buffer is
/// *not* clamped (vim/Helix semantics — empty rows past EOF are allowed).
pub(super) fn scroll_cursor_to_row(
    viewport: &mut ViewportState,
    rm: &mut RowMap<'_>,
    cursor_char: usize,
    target_row: usize,
) {
    let (cursor_pos, _) = rm.locate(cursor_char);
    scroll_back_from(viewport, rm, cursor_pos, target_row);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Put the top of the viewport `rows_above` display rows before `cursor_pos`,
/// saturating at the document's first row.
fn scroll_back_from(
    viewport: &mut ViewportState,
    rm: &mut RowMap<'_>,
    cursor_pos: RowPos,
    rows_above: usize,
) {
    let top = rm.advance(cursor_pos, -(rows_above as isize));
    set_top(viewport, top);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
