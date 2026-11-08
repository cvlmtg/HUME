//! Screen-placement math shared by the three cursor/token-anchored overlays.
//! Nothing here is specific to any one of them.

use hume_engine::layout::PaneGeometry;
use hume_engine::pipeline::{EngineView, PaneSize, ScreenLayout};
use hume_grid::Rect;
use hume_rope::cluster::ClusterStart;

use super::super::{Editor, EditorState};

/// The focused pane on `screen`: its rect, its size, and its gutter and
/// content widths. None of it depends on where the cursor is.
pub(in crate::editor) struct FocusedBox {
    pub(in crate::editor) pane_rect: Rect,
    pub(in crate::editor) size: PaneSize,
    pub(in crate::editor) geometry: PaneGeometry,
}

impl FocusedBox {
    pub(in crate::editor) fn new(
        state: &EditorState,
        view: &EngineView,
        screen: &ScreenLayout,
    ) -> Self {
        let layout = view.focused_layout();
        let size = screen.focused_size(layout);
        Self {
            pane_rect: screen.focused_rect(layout),
            size,
            geometry: state.pane_geometry(view, view.focused_pane(), size),
        }
    }

    /// The text columns the pane gives its content.
    pub(in crate::editor) fn content_width(&self) -> u16 {
        self.geometry.content_width.get().get()
    }
}

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
/// between frames passes `None`, and the walk settles the view first
/// (`frame::settled_pane_view`).
///
/// `screen` is the frame's layout, or [`EditorState::screen_layout`] for a
/// key handler, so a band opened or closed since the last frame counts.
///
/// [`EditorState::screen_layout`]: crate::editor::EditorState::screen_layout
pub(in crate::editor) fn popup_placement(
    ed: &mut Editor,
    screen: &ScreenLayout,
    cursor_cell: Option<(u16, u16)>,
    anchor: ClusterStart,
) -> Option<hume_ui::popup::PopupPlacement> {
    let focused = ed.view.focused_pane();
    let bounds = FocusedBox::new(&ed.state, &ed.view, screen);
    let (pane_rect, size, geometry) = (bounds.pane_rect, bounds.size, bounds.geometry);
    let (content_x, row) = match cursor_cell {
        Some(cell) if anchor == focused_cursor(ed) => cell,
        _ => {
            // Every read of `ed` the map needs resolves before the pane
            // is borrowed mutably; the viewport comes back out of
            // `settled_pane_view`'s own split rather than being held across it.
            let key = ed.state.layout_key(&ed.view, focused, size);
            let Editor { state, view, .. } = ed;
            let (mut dlm, vp) = crate::editor::frame::settled_pane_view(
                &state.buffers,
                &mut state.panes.entries,
                &mut view.panes,
                focused,
                key,
            );
            super::super::cursor::content_pos(size, vp, &mut dlm, anchor)?
        }
    };
    let screen = super::super::mouse::content_pos_to_screen(
        content_x,
        row,
        geometry.gutter_width,
        pane_rect,
    );
    Some(hume_ui::popup::PopupPlacement {
        anchor: screen,
        pane_rect,
        content_width: bounds.content_width(),
    })
}
