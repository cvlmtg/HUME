//! Screen-placement math shared by the three cursor/token-anchored overlays.
//! Nothing here is specific to any one of them.

use hume_rope::cluster::ClusterStart;

use super::super::Editor;

/// The focused pane's primary cursor: the anchor
/// `Editor::resolve_cursor_popup` and
/// [`Editor::sync_menu_view`](crate::editor::Editor::sync_menu_view)
/// pass to [`popup_placement`] (unlike the LSP completion menu, which
/// anchors at the session's token start instead).
pub(in crate::editor) fn focused_cursor(ed: &Editor) -> ClusterStart {
    ed.current_view().primary().head()
}

/// Screen anchor (absolute cell) + containing pane + text-column budget for
/// the focused pane, given an arbitrary buffer position (each caller
/// passes a different `anchor`). `None`
/// when `anchor` isn't currently visible.
///
/// `cursor_cell` is the focused cursor's content cell when the caller
/// already has it: a frame's overlay sync passes the one its scroll step
/// (`scroll_into_view`) resolved, through the same locate/distance walk
/// `content_pos` runs below, since nothing between that step and the sync
/// moves the cursor or the viewport. That saves re-walking the display-line
/// list (a full per-line format in wrap mode). A key handler measuring a box
/// between frames passes `None` and walks the viewport as it stands.
///
/// Lays the panes out against the current bands first: a key handler can
/// measure after a band opened or closed since the last frame, as in a
/// macro, which runs no frame between its keys.
pub(in crate::editor) fn popup_placement(
    ed: &mut Editor,
    cursor_cell: Option<(u16, u16)>,
    anchor: ClusterStart,
) -> Option<hume_ui::popup::PopupPlacement> {
    ed.state.relayout(&mut ed.view);
    let focused = ed.view.focused_pane();
    let pane_rect = ed.view.focused_rect();
    let gutter_w = ed.pane_gutter_width(focused);
    let content_width = ed.view.panes[focused]
        .viewport
        .content_width(gutter_w)
        .get();
    let (content_x, row) = match cursor_cell {
        Some(cell) if anchor == focused_cursor(ed) => cell,
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
            super::super::cursor::content_pos(vp, &mut dlm, anchor)?
        }
    };
    let screen = super::super::mouse::content_pos_to_screen(content_x, row, gutter_w, pane_rect);
    Some(hume_ui::popup::PopupPlacement {
        anchor: screen,
        pane_rect,
        content_width,
    })
}
