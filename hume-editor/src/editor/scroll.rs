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
/// and [`set_top`] are this module's conversion points, so no caller in
/// `editor/` re-derives what `top_row_offset` counts. `hume-engine`'s
/// `pane_render` does its own equivalent conversion on the render path,
/// independently of this module.
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
///
/// Returns the cursor's resulting screen row — its distance below the (possibly
/// just-moved) viewport top. Every arm below already knows that number, so
/// handing it back saves the caller a second walk over the same rows: the
/// stable arm measured it directly, and each scrolling arm gets it from
/// [`scroll_back_from`], which counts the rows it stepped back over. `None`
/// only for a zero-height viewport, which has no screen row to report.
pub(super) fn ensure_cursor_visible(
    viewport: &mut ViewportState,
    rm: &mut RowMap<'_>,
    cursor_pos: RowPos,
    v_margin: usize,
) -> Option<usize> {
    let height = viewport.height as usize;
    if height == 0 {
        return None;
    }
    // `(height - 1) / 2`, not `height / 2`: at an even height, a margin of
    // exactly `height / 2` leaves arm 2's stable window empty (its bounds
    // `margin..height-margin` collapse to a single point), so the two
    // correction arms fight over that one row and rescroll every frame.
    let margin = v_margin.min(height.saturating_sub(1) / 2);
    let top = top_pos(viewport);

    if cursor_pos < top {
        return Some(scroll_back_from(viewport, rm, cursor_pos, margin));
    }

    // `None` means the cursor is more than a viewport's worth of rows below
    // the top, which wants the same correction as any other too-low cursor.
    Some(match rm.distance(top, cursor_pos, height) {
        Some(rows_down) if rows_down < margin => scroll_back_from(viewport, rm, cursor_pos, margin),
        Some(rows_down) if rows_down < height.saturating_sub(margin) => rows_down,
        _ => {
            let target = height.saturating_sub(margin).saturating_sub(1);
            scroll_back_from(viewport, rm, cursor_pos, target)
        }
    })
}

/// Pull the viewport's top onto a row that actually exists.
///
/// Single self-heal chokepoint for staleness: nothing else in the codebase
/// validates a `top_line`/`top_row_offset` write against the block it refers
/// to (`Pane::recall_scroll` restores a saved offset verbatim; an LSP
/// goto-definition jump moves `top_line` without touching `top_row_offset` at
/// all) — and the block a stale address was valid for can shrink or vanish
/// (wrap width change, a virtual-line decoration source removed, a resize) between the
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
    cursor_col: u32,
) {
    const H_MARGIN: usize = 5;

    if rm.is_wrapping() {
        viewport.horizontal_offset = 0;
        return;
    }

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
        viewport.horizontal_offset = cursor_col.saturating_sub(margin) as u32;
    } else if cursor_col >= offset + content_width - margin {
        viewport.horizontal_offset = cursor_col.saturating_sub(content_width - margin - 1) as u32;
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
    let cursor_pos = rm.locate_row(cursor_char);
    scroll_back_from(viewport, rm, cursor_pos, target_row);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Put the top of the viewport `rows_above` display rows before `cursor_pos`,
/// saturating at the document's first row. Returns how many rows it actually
/// stepped back — which, `next`/`prev` being inverses, is the cursor's screen
/// row under the new top.
fn scroll_back_from(
    viewport: &mut ViewportState,
    rm: &mut RowMap<'_>,
    cursor_pos: RowPos,
    rows_above: usize,
) -> usize {
    let (top, stepped) = rm.advance_counted(cursor_pos, -(rows_above as isize));
    set_top(viewport, top);
    stepped
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
