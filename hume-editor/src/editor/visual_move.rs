//! Visual-line movement commands (`j`/`k` with soft-wrap).
//!
//! When soft-wrap is active, `j`/`k` move by one display row rather than one
//! buffer line. These commands need a `RowMap` — unavailable in the pure
//! `(&Text, SelectionSet) -> SelectionSet` motion signature — so they live here
//! instead of `hume-ops`'s `motion` module.

use hume_editing::selection::{DisplayColOrigin, Selection, StickyDisplayCol};
use hume_editing::text::Text;
use hume_engine::pipeline::EngineView;
use hume_engine::rows::{DisplayColTarget, RowKind, RowMap};
use hume_ops::MotionMode;
use hume_ops::text_object::{
    apply_nearest_word_result, cmd_select_word_nearest_on_line, nearest_word_on_line,
};

use super::commands::{apply_focused_motion, effective_wrap_mode, focused_buffer_id, pane_row_map};
use super::{EditorState, doc_ops};
use crate::editor::error::CommandError;

// ---------------------------------------------------------------------------
// Vertical movement
// ---------------------------------------------------------------------------

/// Move `head` by `count` display rows, landing on the last content row
/// reached (or staying put if the document's edge came first).
///
/// `content_only`: when `true`, only content rows count against `count` —
/// virtual rows are neither a cost nor a landing spot, so a virtual-line
/// decoration source's rows never swallow a `j`/`k` keystroke. When
/// `false`, every display row counts, virtual ones included — for callers
/// whose `count` is already a display-row measurement of something else
/// (the mouse wheel's or page-scroll's own viewport delta), which it has to
/// track 1:1 so the cursor stays at roughly the same relative screen row.
fn move_vertical(
    rm: &mut RowMap<'_>,
    head: usize,
    down: bool,
    count: usize,
    target_display_col: u32,
    content_only: bool,
) -> usize {
    let start = rm.locate_row(head);
    let mut pos = start;
    let mut last_content = start;
    let mut remaining = count;

    while remaining > 0 {
        let Some(next) = (if down { rm.next(pos) } else { rm.prev(pos) }) else {
            break; // document start/end — clamp to the last row reached
        };
        pos = next;
        let is_content = matches!(rm.kind(pos), RowKind::Content(_));
        if is_content {
            last_content = pos;
        }
        if !content_only || is_content {
            remaining -= 1;
        }
    }

    if last_content == start {
        // Already on the document's first/last content row: leave the head
        // exactly where it was rather than snapping it to `target_display_col`.
        return head;
    }
    rm.char_at(
        last_content,
        target_display_col,
        DisplayColTarget::NearestContent,
    )
}

/// Move `head` by `count` buffer lines, landing on the target line's own
/// line-relative display column (`RowMap::char_at_line_display_col`).
///
/// Distinct from `move_vertical`: a numeric-prefixed vertical move (`9j`) is a
/// direct line-index jump matching relative-line-number gutters, not a
/// display-row walk — virtual rows and wrap rows are both irrelevant to it.
fn move_buffer_line(
    rm: &mut RowMap<'_>,
    text: &Text,
    head: usize,
    down: bool,
    count: usize,
    target_line_display_col: u32,
) -> usize {
    let line = text.char_to_line(head);
    let target_line = if down {
        // On the last content line, line + count would be the phantom
        // trailing line (the structural \n) — clamp there is nothing past it.
        line.saturating_add(count).min(text.last_content_line())
    } else {
        line.saturating_sub(count)
    };
    if target_line == line {
        return head; // already at the document's first/last content line
    }
    rm.char_at_line_display_col(
        target_line,
        target_line_display_col,
        DisplayColTarget::NearestContent,
    )
}

/// How `apply_visual_vertical`'s `count` should be interpreted.
pub(super) enum VerticalUnit {
    /// `count` buffer lines — `j`/`k` with an explicit numeric prefix
    /// (matches relative-line-number gutters even while wrapping).
    BufferLine,
    /// `count` real content rows; virtual rows are free — plain `j`/`k` with
    /// no explicit count.
    ContentRow,
    /// `count` display rows, virtual rows included — mouse wheel and
    /// page/half-page scroll.
    ScreenRow,
}

/// Shared core for the visual-line movement EditorCmds and screen-relative
/// scroll commands (page/half-page, mouse wheel).
pub(super) fn apply_visual_vertical(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    down: bool,
    mode: MotionMode,
    unit: VerticalUnit,
) {
    let focused = state.focused_pane_id;
    // Every unit now resolves its column through `RowMap` — `ContentRow`/
    // `ScreenRow` via `move_vertical`'s row walk, `BufferLine` (`9j`/`9k`) via
    // `move_buffer_line`'s direct line jump — so all three latch a column
    // from the same authority. `DisplayColOrigin` still distinguishes what
    // the column is measured *from*: a wrapped `DisplayRow` latch is
    // row-relative and a `BufferLine` latch is line-relative, and the two
    // coincide only when nothing wraps (see `DisplayColOrigin`'s own doc).
    let content_only = !matches!(unit, VerticalUnit::ScreenRow);

    let buf_id = focused_buffer_id(state, view);
    // A latch this path wrote is tagged by whether wrapping was on at the
    // time — see `DisplayColOrigin`. Resolved once per call, before the row
    // map borrows the pane, since neither borrow is mutable.
    let wrapping = effective_wrap_mode(
        state.buffers.get(buf_id),
        &state.settings,
        &view.panes[focused],
    )
    .is_wrapping();
    let origin = if wrapping && !matches!(unit, VerticalUnit::BufferLine) {
        DisplayColOrigin::DisplayRow
    } else {
        DisplayColOrigin::BufferLine
    };
    let is_buffer_line = matches!(unit, VerticalUnit::BufferLine);
    let target_display_cols = &mut state.visual_move_target_display_cols;
    target_display_cols.clear();
    let mut rm = pane_row_map(
        state.buffers.get(buf_id),
        &state.settings,
        &view.panes[focused],
        &mut state.motion_format_scratch,
    );

    // Not `apply_focused_motion`: the closure also captures the row map and the
    // sticky-column buffer, disjoint fields of `state` that must be borrowed
    // separately from `state.panes`.
    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        focused,
        buf_id,
        |text, sels| {
            // Pass 1: resolve each selection's sticky display column. A latch
            // tagged for this call's own origin is reused as-is; one tagged
            // for the other origin is a different quantity (see
            // `DisplayColOrigin`) and is re-derived instead, the same as no
            // latch at all — line-relative for `BufferLine`, row-relative
            // otherwise, mirroring which one `move_buffer_line`/`move_vertical`
            // below is about to consume.
            let current_wrap_width = rm.resolved_wrap_width();
            target_display_cols.extend(sels.iter_sorted().map(
                |sel| match sel.sticky_display_col() {
                    Some(sticky)
                        if sticky.origin == origin
                            && (origin == DisplayColOrigin::BufferLine
                                || sticky.wrap_width == current_wrap_width) =>
                    {
                        sticky.display_col
                    }
                    _ if is_buffer_line => rm.line_display_col(sel.head()),
                    _ => rm.locate(sel.head()).1,
                },
            ));

            // Pass 2: move each selection, preserving the sticky column so
            // consecutive presses in the same family reuse it.
            let mut display_col_iter = target_display_cols.iter();
            sels.map(|sel| {
                let &target_display_col =
                    display_col_iter.next().expect("one column per selection");
                let head = if is_buffer_line {
                    move_buffer_line(&mut rm, text, sel.head(), down, count, target_display_col)
                } else {
                    move_vertical(
                        &mut rm,
                        sel.head(),
                        down,
                        count,
                        target_display_col,
                        content_only,
                    )
                };
                let anchor = if mode == MotionMode::Extend {
                    sel.anchor()
                } else {
                    head
                };
                Selection::with_sticky_display_col(
                    anchor,
                    head,
                    StickyDisplayCol {
                        display_col: target_display_col,
                        origin,
                        // Meaningless for `BufferLine` (see `StickyDisplayCol`'s
                        // doc) — stored as `None` uniformly rather than
                        // whatever the wrap mode happened to resolve to at
                        // write time, so equality on the latch doesn't
                        // depend on an origin-irrelevant field.
                        wrap_width: if origin == DisplayColOrigin::DisplayRow {
                            current_wrap_width
                        } else {
                            None
                        },
                    },
                )
            })
        },
    );
}

// ---------------------------------------------------------------------------
// Public commands
// ---------------------------------------------------------------------------

pub(super) fn cmd_visual_move_down(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    // A count typed by the user (e.g. `9j`) means "9 buffer lines" — matching
    // relative-line-number gutters — even when soft-wrap is on.
    let unit = if state.explicit_count {
        VerticalUnit::BufferLine
    } else {
        VerticalUnit::ContentRow
    };
    apply_visual_vertical(state, view, count, true, mode, unit);
    Ok(())
}

pub(super) fn cmd_visual_move_up(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let unit = if state.explicit_count {
        VerticalUnit::BufferLine
    } else {
        VerticalUnit::ContentRow
    };
    apply_visual_vertical(state, view, count, false, mode, unit);
    Ok(())
}

/// Wrap-aware variant of `select-word-nearest-on-line`.
///
/// When wrap is active, scopes the nearest-word search to the selection
/// anchor's current visual row rather than the full buffer line — matching
/// `cmd_select_word_nearest_on_line`'s own use of `sel.anchor()`. This
/// prevents the search from finding words that live on an adjacent visual
/// row when the anchor lands on leading whitespace near a wrap boundary —
/// the failure mode that causes `j`/`k` bindings to oscillate in place.
///
/// Falls back to `cmd_select_word_nearest_on_line` (buffer-line bounds) when
/// wrap is off, producing identical behaviour.
pub(super) fn cmd_visual_select_word_nearest_on_line(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = focused_buffer_id(state, view);
    let doc = state.buffers.get(buf_id);
    let around = doc.overrides.word_selects_whitespace(&state.settings);

    if !effective_wrap_mode(doc, &state.settings, &view.panes[state.focused_pane_id]).is_wrapping()
    {
        apply_focused_motion(state, view, |buf, sels| {
            cmd_select_word_nearest_on_line(buf, sels, 0, mode, around)
        });
        return Ok(());
    }

    let focused = state.focused_pane_id;
    let mut rm = pane_row_map(
        state.buffers.get(buf_id),
        &state.settings,
        &view.panes[focused],
        &mut state.motion_format_scratch,
    );

    // Not `apply_focused_motion`: the closure also captures the row map.
    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        focused,
        buf_id,
        |text, sels| {
            let new_sels = sels.map(|sel| {
                let pos = rm.locate_row(sel.anchor());
                let (line_start, line_end_excl) =
                    rm.content_row_char_bounds(pos).unwrap_or_else(|| {
                        let buf_line = text.char_to_line(sel.anchor());
                        let ls = text.line_to_char(buf_line);
                        let le = hume_editing::lines::line_end_exclusive(text, buf_line);
                        (ls, le)
                    });

                let found =
                    nearest_word_on_line(text, sel.anchor(), line_start, line_end_excl, around);
                apply_nearest_word_result(sel, found, mode)
            });
            new_sels.debug_assert_valid(text);
            new_sels
        },
    );

    Ok(())
}
