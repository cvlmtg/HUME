//! Screen-placement math shared by the three cursor/token-anchored overlays —
//! [`super::popup::sync_popup_view`], [`super::menu::sync_menu_view`], and
//! [`super::completion::sync_completion_menu_view`] — nothing here is
//! specific to any one of them.

use hume_engine::pipeline::RenderContext;
use hume_rope::offset::CharOffset;

use super::super::Editor;

/// The focused pane's primary cursor position — the anchor char
/// [`super::popup::sync_popup_view`] and [`super::menu::sync_menu_view`]
/// pass to [`popup_placement`] (unlike the LSP completion menu, which
/// anchors at the session's token-start char instead, via a
/// separately-computed `anchor_char`).
pub(in crate::editor) fn focused_cursor_char(ed: &Editor) -> CharOffset {
    ed.current_selections().primary().head()
}

/// Screen anchor (absolute cell) + containing pane + text-column budget for
/// the focused pane, given an arbitrary buffer char position — shared by
/// [`super::popup::sync_popup_view`], [`super::menu::sync_menu_view`], and
/// the LSP completion menu (each passes a different `anchor_char`). `None`
/// when the pane has no rect yet or `anchor_char` isn't currently visible.
pub(in crate::editor) fn popup_placement(
    ed: &mut Editor,
    ctx: &mut RenderContext,
    anchor_char: CharOffset,
) -> Option<hume_ui::popup::PopupPlacement> {
    let focused = ed.state.focus.id();
    let pane_rect = ed.view.pane_rect(focused)?;
    let gutter_w = ed.pane_gutter_width(focused);
    let content_width = pane_rect.width.saturating_sub(gutter_w);
    // The scroll step (`scroll_into_view`) already resolved the focused cursor's
    // screen cell this frame, via the same locate/distance walk
    // `content_pos` runs below — nothing between the scroll step and this
    // overlay sync moves the cursor or the viewport, so the two callers anchored
    // at the live cursor (`sync_popup_view`, `sync_menu_view`) can reuse it instead of
    // re-walking the display-line list (a full per-line format in wrap mode).
    let (content_x, row) = match ctx.cursor_content_pos {
        Some(cell) if anchor_char == focused_cursor_char(ed) => cell,
        _ => {
            // Every read of `ed` the map needs resolves before the pane
            // is borrowed mutably; the viewport comes back out of
            // `pane_display_lines`'s own split rather than being held across it.
            let bid = ed.focused_buffer_id();
            let key = ed.state.format_key(&ed.view.panes[focused]);
            let Editor { state, view, .. } = ed;
            let (mut dlm, vp) = super::super::commands::pane_display_lines(
                state.buffers.get(bid),
                &mut view.panes[focused],
                key,
            );
            super::super::cursor::content_pos(vp, &mut dlm, anchor_char)?
        }
    };
    let anchor = super::super::mouse::content_pos_to_screen(content_x, row, gutter_w, pane_rect);
    Some(hume_ui::popup::PopupPlacement {
        anchor,
        pane_rect,
        content_width,
    })
}
