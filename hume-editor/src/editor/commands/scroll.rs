use hume_core::{Count, PaneId};
use hume_engine::display_lines::{DisplayColTarget, DisplayLineMap, DisplayLinePos, carry};
use hume_engine::pane::{ViewGeometry, ViewLayouts, Viewport};
use hume_engine::pipeline::{EngineView, PanePool, PaneSize};
use hume_ops::MotionMode;

use super::super::EditorState;
use super::super::buffer::store::BufferStore;
use super::super::doc_ops;
use super::super::frame::{reveal_due_cursor, settled_pane_view, settled_view_layouts};
use super::super::geometry::LayoutKey;
use super::super::pane_state::PaneEntries;
use super::{CommandPane, sticky_landing};
use crate::editor::error::CommandError;
use hume_editing::selection::Selection;
use hume_rope::cluster::ClusterStart;
use hume_rope::column::DisplayLineCol;

// ── Moving the view ───────────────────────────────────────────────────────────

/// Move pane `pid`'s view between frames, laid out at `size`: `run`
/// settles the pane, so the move starts from the view the last cursor move
/// would have drawn, whether or not a frame drew it, and moves it. The
/// reveal the move leaves due runs after it, so `PaneBufferState::parked`
/// describes the view the move ends on. A pane with no rows has no view to
/// move, and `run` does not run.
fn view_move(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    size: PaneSize,
    run: impl FnOnce(&BufferStore, &mut PaneEntries, &mut PanePool, &LayoutKey, ViewGeometry<'_>),
) {
    let key = state.layout_key(view, pid, size);
    let buffer = state.buffers.get(view.panes[pid].buffer_id());
    let Some(geo) = key.geometry(buffer.syntax_source()) else {
        return;
    };
    run(
        &state.buffers,
        &mut state.panes.entries,
        &mut view.panes,
        &key,
        geo,
    );
    // Built after the move: in a buffer with conceals the moved selections
    // reveal other lines, and recording the pre-move key would read as a
    // layout change and reveal again before the next input.
    let moved = state.layout_key(view, pid, size);
    reveal_due_cursor(
        &state.buffers,
        &mut state.panes.entries,
        &mut view.panes,
        pid,
        &moved,
    );
}

/// What a view move gets: the pane's settled drawn layout and viewport, and
/// its scroll geometry.
struct ViewMove<'a> {
    drawn: DisplayLineMap<'a>,
    viewport: &'a mut Viewport,
    geo: ViewGeometry<'a>,
}

/// Move pane `pid`'s view with `f`, leaving every selection where it is.
fn move_view(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    size: PaneSize,
    f: impl FnOnce(ViewMove<'_>),
) {
    view_move(
        state,
        view,
        pid,
        size,
        |buffers, entries, panes, key, geo| {
            let (drawn, viewport) = settled_pane_view(buffers, entries, panes, pid, key);
            f(ViewMove {
                drawn,
                viewport,
                geo,
            });
        },
    );
}

/// What a view move that carries the cursors gets beyond a [`ViewMove`]:
/// the pane's raw layout ([`ViewLayouts::raw`]) and the selection stores,
/// free to write while the layouts are held.
struct CarryMove<'a> {
    view: ViewMove<'a>,
    raw: Option<DisplayLineMap<'a>>,
    buffers: &'a BufferStore,
    entries: &'a mut PaneEntries,
}

/// Move pane `pid`'s view with `f`, which carries the cursors with it.
fn carry_view(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    size: PaneSize,
    f: impl FnOnce(CarryMove<'_>),
) {
    view_move(
        state,
        view,
        pid,
        size,
        |buffers, entries, panes, key, geo| {
            let ViewLayouts {
                drawn,
                raw,
                viewport,
            } = settled_view_layouts(buffers, entries, panes, pid, key);
            f(CarryMove {
                view: ViewMove {
                    drawn,
                    viewport,
                    geo,
                },
                raw,
                buffers,
                entries,
            });
        },
    );
}

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses the pane's height (or half of it) as the move count rather than the user's
// numeric prefix.

/// Scroll pane `pid` `count` display lines and carry every cursor the
/// same distance, so the cursor keeps its screen row.
///
/// The single implementation behind the wheel, `Ctrl-d`/`Ctrl-u` and
/// `PageDown`/`PageUp`: they differ only in `count` and which pane they pass
/// (always `view.focused_pane()`, except the wheel, which hit-tests the pointer
/// against the panes the last frame drew; see `editor/mouse.rs`). `Viewport::scroll_by`
/// is what makes a scroll move the view even while the cursor is still
/// inside it (a `Ctrl-d` from the top of a file); `carry`, called per
/// selection below with the same requested `delta` and the viewport's own
/// post-scroll `top` (walking its own bound independently of `scroll_by`'s;
/// see `hume_engine::display_lines::scroll`'s module doc for why) is what
/// keeps the cursor at the same relative position (or, failing that, lands
/// it back inside the scroll-margin band) instead of being snapped back by
/// the reveal that follows. See `carry`'s own doc for why a selection it
/// can't place stays untouched rather than collapsing.
///
/// The scroll and `carry` walk the drawn layout, the rows on screen. The
/// cursor lands on the text of the row `carry` picks, at the column `j`/`k`
/// would use: see [`land`].
pub(in crate::editor) fn scroll_view(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    size: PaneSize,
    count: usize,
    down: bool,
    mode: MotionMode,
) {
    let buf_id = view.panes[pid].buffer_id();
    let landing = sticky_landing(state, pid, buf_id);
    let delta = if down {
        count as isize
    } else {
        -(count as isize)
    };
    carry_view(
        state,
        view,
        pid,
        size,
        |CarryMove {
             view:
                 ViewMove {
                     mut drawn,
                     viewport,
                     geo,
                 },
             mut raw,
             buffers,
             entries,
         }| {
            viewport.scroll_by(&mut drawn, geo, delta);
            let top = geo.carry_top(&mut drawn, viewport.top());
            // `apply_doc_motion`'s own head-before/after comparison is what
            // raises `PaneBufferState::reveal_pending` here. No separate pin
            // to track: a scroll that couldn't carry a selection anywhere
            // leaves that selection's head unchanged, which is the case the
            // funnel already treats as "nothing to reveal".
            doc_ops::apply_doc_motion(buffers, entries, pid, buf_id, |st| {
                st.map(|sel| {
                    // The head's line is revealed, so its column is the
                    // same in both layouts.
                    let (head_pos, target_col) = drawn.locate(sel.head());
                    // `None`: parked behind a virtual block or at a document edge.
                    let Some(landed) = carry(&mut drawn, geo, top, head_pos, delta) else {
                        return sel.selection();
                    };
                    let new_head = land(&mut drawn, raw.as_mut(), landed, target_col, landing);
                    let anchor = if mode == MotionMode::Extend {
                        sel.anchor()
                    } else {
                        new_head
                    };
                    Selection::new(anchor, new_head)
                })
            });
        },
    );
}

/// The cluster a cursor landing on drawn display line `landed` at
/// `target_col` takes, resolved on `raw`, or on `drawn` when `raw` is
/// `None`. The column resolves on the raw layout, as `j`/`k` place it,
/// because the line the cursor lands on reveals once it holds the cursor.
/// A row number means different text in the two layouts once conceals
/// change how a line wraps, so the row crosses as the cluster it starts
/// with, and the landing stays within the row's clusters.
fn land(
    drawn: &mut DisplayLineMap<'_>,
    raw: Option<&mut DisplayLineMap<'_>>,
    landed: DisplayLinePos,
    target_col: DisplayLineCol,
    landing: DisplayColTarget,
) -> ClusterStart {
    let Some(raw) = raw else {
        return drawn.char_at(landed, target_col, landing);
    };
    // Unwrapped, a line is one display line in either layout.
    let row = if drawn.is_wrapping() {
        drawn.content_display_line_clusters(landed)
    } else {
        None
    };
    let Some(row) = row else {
        let raw_pos = raw.first_content_display_line(landed.line);
        return raw.char_at(raw_pos, target_col, landing);
    };
    // The raw display line holding the row's first cluster can start
    // before the row or run past it.
    let raw_pos = raw.locate_display_line(row.start());
    raw.char_at(raw_pos, target_col, landing)
        .clamp(row.start(), row.last())
}

/// How far `Ctrl-d`/`Ctrl-u` move given `visible_rows` rows on screen. The
/// buffer viewport, the bottom drawer, the fuzzy picker, and a scrollable
/// popup all page by this same step, so a browsed drawer or picker feels
/// like a scrolled pane under the same keys.
///
/// Floors rather than rounds up (`9` visible rows steps `4`, not `5`) to
/// match the conventional buffer half-page; `.max(1)` keeps a one- or
/// two-row view pageable instead of stalling on a truncated division.
/// `0` stays `0` rather than climbing to `1`: a view that shows no rows (a
/// collapsed pane, a terminal too short for any drawer row or for the
/// picker panel) must not page, not even by one row.
pub(in crate::editor) fn half_page(visible_rows: usize) -> usize {
    if visible_rows == 0 {
        return 0;
    }
    (visible_rows / 2).max(1)
}

fn scroll_page(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    mode: MotionMode,
    half: bool,
    down: bool,
) -> Result<(), CommandError> {
    let size = state.pane_size(view, pid);
    let height = usize::from(size.height());
    let count = if half { half_page(height) } else { height };
    scroll_view(state, view, pid, size, count, down, mode);
    Ok(())
}

pub(in crate::editor) fn cmd_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, t.pid(), mode, false, true)
}
pub(in crate::editor) fn cmd_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, t.pid(), mode, false, false)
}
pub(in crate::editor) fn cmd_half_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, t.pid(), mode, true, true)
}
pub(in crate::editor) fn cmd_half_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, t.pid(), mode, true, false)
}

// ── View-trie scroll (z z / z k / z j) ────────────────────────────────────────

fn cmd_view_scroll_to_display_line(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    size: PaneSize,
    target_display_line: usize,
) {
    let buf_id = view.panes[pid].buffer_id();
    let cursor_char = state
        .panes
        .entries
        .shown(&view.panes, pid)
        .view(state.buffers.get(buf_id).text())
        .primary()
        .head();
    move_view(
        state,
        view,
        pid,
        size,
        |ViewMove {
             mut drawn,
             viewport,
             geo,
         }| {
            let cursor_pos = drawn.locate_display_line(cursor_char);
            viewport.align(&mut drawn, geo, cursor_pos, target_display_line);
        },
    );
}

/// Center the head in the viewport, like `z z`. Infallible core shared by
/// [`cmd_view_center`] (the registered `z z` command) and any other caller
/// that wants the same effect without going through a native `EditorCmd`'s
/// `Result`.
pub(in crate::editor) fn view_center(state: &mut EditorState, view: &mut EngineView, pid: PaneId) {
    let size = state.pane_size(view, pid);
    let target = usize::from(size.height()) / 2;
    cmd_view_scroll_to_display_line(state, view, pid, size, target);
}

/// Pin the head at the viewport's top display line, like `z k`. Infallible core
/// of [`cmd_view_top`].
pub(in crate::editor::commands) fn view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
) {
    let size = state.pane_size(view, pid);
    cmd_view_scroll_to_display_line(state, view, pid, size, 0);
}

pub(in crate::editor) fn cmd_view_center(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    view_center(state, view, t.pid());
    Ok(())
}

pub(in crate::editor) fn cmd_view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    view_top(state, view, t.pid());
    Ok(())
}

pub(in crate::editor) fn cmd_view_bottom(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let size = state.pane_size(view, t.pid());
    let target = usize::from(size.height()).saturating_sub(1);
    cmd_view_scroll_to_display_line(state, view, t.pid(), size, target);
    Ok(())
}
