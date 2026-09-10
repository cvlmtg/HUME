//! Scroll logic for the engine-based viewport.
//!
//! Every routine here is a thin consumer of `hume_engine::display_lines::DisplayLineMap`, the
//! one authority on the document's display-line list. Scrolling is then just
//! arithmetic on display-line addresses: "put the cursor's display line `n`
//! display lines below the top" is `advance(cursor, -n)`, in either wrap
//! mode, with or without virtual display lines.

use hume_engine::display_lines::{DisplayLineMap, DisplayLinePos};
use hume_engine::pane::ViewportState;
use hume_rope::column::DisplayLineCol;

// ---------------------------------------------------------------------------
// Viewport ↔ display-line address
// ---------------------------------------------------------------------------

/// The viewport's top display line as a `DisplayLinePos` address.
///
/// `ViewportState`'s `top_line`/`top_slot` pair *is* a `DisplayLinePos` — this
/// and [`set_top`] are this module's conversion points, so no caller in
/// `editor/` re-derives what `top_slot` counts. `hume-engine`'s
/// `pane_render` does its own equivalent conversion on the render path,
/// independently of this module.
pub(super) fn top_pos(viewport: &ViewportState) -> DisplayLinePos {
    DisplayLinePos::new(viewport.top_line, viewport.top_slot as usize)
}

/// Move the viewport's top to `pos`.
pub(super) fn set_top(viewport: &mut ViewportState, pos: DisplayLinePos) {
    viewport.top_line = pos.line;
    viewport.top_slot = u16::try_from(pos.slot).unwrap_or(u16::MAX);
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Adjust the viewport so the cursor's display line is visible with `v_margin`
/// display lines of look-ahead above and below it.
///
/// Returns the cursor's resulting screen row — its distance below the (possibly
/// just-moved) viewport top. Every arm below already knows that number, so
/// handing it back saves the caller a second walk over the same display
/// lines: the stable arm measured it directly, and each scrolling arm gets
/// it from [`scroll_back_from`], which counts the display lines it stepped
/// back over. `None` only for a zero-height viewport, which has no screen
/// row to report.
pub(super) fn ensure_cursor_visible(
    viewport: &mut ViewportState,
    dlm: &mut DisplayLineMap<'_>,
    cursor_pos: DisplayLinePos,
    v_margin: usize,
) -> Option<usize> {
    let height = viewport.height as usize;
    if height == 0 {
        return None;
    }
    // `(height - 1) / 2`, not `height / 2`: at an even height, a margin of
    // exactly `height / 2` leaves arm 2's stable window empty (its bounds
    // `margin..height-margin` collapse to a single point), so the two
    // correction arms fight over that one display line and rescroll every frame.
    let margin = v_margin.min(height.saturating_sub(1) / 2);
    let top = top_pos(viewport);

    if cursor_pos < top {
        return Some(scroll_back_from(viewport, dlm, cursor_pos, margin));
    }

    // `None` means the cursor is more than a viewport's worth of display
    // lines below the top, which wants the same correction as any other
    // too-low cursor.
    //
    // Capped at `target` rather than `height`: `display_lines_down < height -
    // margin` is the only comparison this result ever feeds (below), so
    // `Some(k)` for any `k >= target + 1` is indistinguishable from `None` at
    // the call site — walking past `target` under wrap only pays for
    // `format_buffer_line` calls whose answer nothing inspects.
    let target = height.saturating_sub(margin).saturating_sub(1);
    Some(match dlm.distance(top, cursor_pos, target) {
        Some(display_lines_down) if display_lines_down < margin => {
            scroll_back_from(viewport, dlm, cursor_pos, margin)
        }
        Some(display_lines_down) if display_lines_down < height.saturating_sub(margin) => {
            display_lines_down
        }
        _ => scroll_back_from(viewport, dlm, cursor_pos, target),
    })
}

/// Pull the viewport's top onto a display line that actually exists.
///
/// Single self-heal chokepoint for staleness: nothing else in the codebase
/// validates a `top_line`/`top_slot` write against the block it refers
/// to (`Pane::recall_scroll` restores a saved offset verbatim; an LSP
/// goto-definition jump moves `top_line` without touching `top_slot` at
/// all) — and the block a stale address was valid for can shrink or vanish
/// (wrap width change, a virtual-line decoration source removed, a resize) between the
/// write and the next read. Call once per pane per frame, before
/// `ensure_cursor_visible`, so every other write site can stay unvalidated.
pub(super) fn clamp_viewport_top(viewport: &mut ViewportState, dlm: &mut DisplayLineMap<'_>) {
    let clamped = dlm.clamp(top_pos(viewport));
    set_top(viewport, clamped);
}

/// Adjust `viewport.horizontal_offset` so the cursor's display column stays
/// visible. Wrapping modes have no horizontal scroll, so the offset is forced
/// to 0 there. The horizontal margin is fixed — `scrolloff` governs only the
/// vertical axis.
pub(super) fn ensure_cursor_visible_horizontal(
    viewport: &mut ViewportState,
    dlm: &mut DisplayLineMap<'_>,
    cursor_display_col: DisplayLineCol,
) {
    const H_MARGIN: usize = 5;

    if dlm.is_wrapping() {
        viewport.horizontal_offset = DisplayLineCol::new(0);
        return;
    }

    // The rest of this function mixes the column with plain margin/width
    // counts throughout, so it drops to `.get()`'s bare `u32` at the top
    // rather than threading `DisplayLineCol` arithmetic through — the same
    // trade-off `align_selections` makes for the same reason.
    let cursor_display_col = cursor_display_col.get() as usize;
    // `locate`'s column is content-relative (the gutter isn't part of it),
    // so the margin must compare against the content width the map itself
    // was built with — not `viewport.width`, which still includes the
    // gutter and so under-counts how many columns are actually visible.
    let content_width = dlm.content_width() as usize;
    if content_width == 0 {
        return;
    }

    let margin = H_MARGIN.min(content_width / 2);
    let offset = viewport.horizontal_offset.get() as usize;

    if cursor_display_col < offset + margin {
        viewport.horizontal_offset =
            DisplayLineCol::new(cursor_display_col.saturating_sub(margin) as u32);
    } else if cursor_display_col >= offset + content_width - margin {
        viewport.horizontal_offset = DisplayLineCol::new(
            cursor_display_col.saturating_sub(content_width - margin - 1) as u32,
        );
    }
}

/// Scroll so the cursor's display line lands `target_display_line` display
/// lines below the top of the viewport. Used by the `z z`/`z k`/`z j` view
/// commands.
///
/// Top-of-buffer is clamped to the document's first display line;
/// bottom-of-buffer is *not* clamped (vim/Helix semantics — empty display
/// lines past EOF are allowed).
pub(super) fn scroll_cursor_to_display_line(
    viewport: &mut ViewportState,
    dlm: &mut DisplayLineMap<'_>,
    cursor_char: hume_rope::offset::CharOffset,
    target_display_line: usize,
) {
    let cursor_pos = dlm.locate_display_line(cursor_char);
    scroll_back_from(viewport, dlm, cursor_pos, target_display_line);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Put the top of the viewport `display_lines_above` display lines before
/// `cursor_pos`, saturating at the document's first display line. Returns
/// how many display lines it actually stepped back — which, `next`/`prev`
/// being inverses, is the cursor's screen row under the new top.
fn scroll_back_from(
    viewport: &mut ViewportState,
    dlm: &mut DisplayLineMap<'_>,
    cursor_pos: DisplayLinePos,
    display_lines_above: usize,
) -> usize {
    let (top, stepped) = dlm.advance_counted(cursor_pos, -(display_lines_above as isize));
    set_top(viewport, top);
    stepped
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
