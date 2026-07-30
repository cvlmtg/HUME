//! Visual-line movement commands (`j`/`k` with soft-wrap).
//!
//! When soft-wrap is active, `j`/`k` move by one display row rather than one
//! buffer line. These commands need a `RowMap` — unavailable in the pure
//! `(&Text, SelectionSet) -> SelectionSet` motion signature — so they live here
//! instead of `ops/motion`.

use crate::ops::MotionMode;
use crate::ops::motion::{cmd_move_down, cmd_move_up};
use crate::ops::text_object::{
    apply_nearest_word_result, cmd_select_word_nearest_on_line, nearest_word_on_line,
};
use hume_editing::selection::Selection;
use hume_engine::pipeline::EngineView;
use hume_engine::rows::{ColTarget, RowKind, RowMap};

use super::commands::{apply_focused_motion, focused_buffer_id, pane_row_map};
use super::{EditorState, doc_ops};
use crate::editor::error::CommandError;

// ---------------------------------------------------------------------------
// Vertical movement
// ---------------------------------------------------------------------------

/// Which display rows a vertical move charges to its budget.
#[derive(Copy, Clone)]
enum RowBudget {
    /// Only content rows — virtual rows are neither a cost nor a landing spot,
    /// so a `VirtualLineSource`'s rows never swallow a `j`/`k` keystroke.
    ContentOnly,
    /// Every display row, virtual ones included. For callers whose `count` is
    /// already a display-row measurement of something else — the mouse wheel's
    /// or page-scroll's own viewport delta — which it has to track 1:1 so the
    /// cursor stays at roughly the same relative screen row.
    EveryRow,
}

/// Move `head` by `count` display rows, landing on the last content row
/// reached (or staying put if the document's edge came first).
fn move_vertical(
    rm: &mut RowMap<'_>,
    head: usize,
    down: bool,
    count: usize,
    target_col: u16,
    budget: RowBudget,
) -> usize {
    let (start, _) = rm.locate(head);
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
        match budget {
            RowBudget::ContentOnly if !is_content => {}
            _ => remaining -= 1,
        }
    }

    if last_content == start {
        // Already on the document's first/last content row: leave the head
        // exactly where it was rather than snapping it to `target_col`.
        return head;
    }
    rm.char_at(last_content, target_col, ColTarget::NearestContent)
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
    // Only an explicit count (`9j`) takes the pure buffer-line motion, to
    // match relative-line-number gutters even while wrapping. A bare `j`/`k`
    // always goes through `move_vertical` below so it shares one column
    // model — the sticky *display* column `Selection::horiz` is documented
    // for — with page/half-page scroll and the mouse wheel (`ScreenRow`),
    // which must preserve display columns across virtual rows regardless of
    // wrap mode. The cost in no-wrap mode is a per-press format of the
    // cursor's line instead of pure rope arithmetic; `move_down_inner`'s
    // char-offset column model is now reached only via `BufferLine`.
    let use_buffer_line_motion = matches!(unit, VerticalUnit::BufferLine);
    if use_buffer_line_motion {
        let motion = if down { cmd_move_down } else { cmd_move_up };
        apply_focused_motion(state, view, |b, s| motion(b, s, count, mode));
        return;
    }
    let budget = if matches!(unit, VerticalUnit::ScreenRow) {
        RowBudget::EveryRow
    } else {
        RowBudget::ContentOnly
    };

    let buf_id = focused_buffer_id(state, view);
    let target_cols = &mut state.visual_move_target_cols;
    target_cols.clear();
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
        |_text, sels| {
            // Pass 1: resolve each selection's sticky display column from
            // sel.horiz, computing it fresh on the first j/k press.
            target_cols.extend(sels.iter_sorted().map(|sel| {
                sel.horiz()
                    .map_or_else(|| rm.locate(sel.head()).1, |col| col as u16)
            }));

            // Pass 2: move each selection, preserving the sticky column so
            // consecutive j/k presses reuse it.
            let mut col_iter = target_cols.iter();
            sels.map(|sel| {
                let &target_col = col_iter.next().expect("one column per selection");
                let head = move_vertical(&mut rm, sel.head(), down, count, target_col, budget);
                let anchor = if mode == MotionMode::Extend {
                    sel.anchor()
                } else {
                    head
                };
                Selection::with_horiz(anchor, head, target_col as u32)
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
/// When wrap is active, scopes the nearest-word search to the head's current
/// visual row rather than the full buffer line. This prevents the search from
/// finding words that live on an adjacent visual row when the head lands on
/// leading whitespace near a wrap boundary — the failure mode that causes
/// `j`/`k` bindings to oscillate in place.
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
    let around = state
        .buffers
        .get(buf_id)
        .overrides
        .word_selects_whitespace(&state.settings);

    if !view.panes[state.focused_pane_id].wrap_mode.is_wrapping() {
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
                let (pos, _) = rm.locate(sel.anchor());
                let (line_start, line_end_excl) =
                    rm.content_row_char_bounds(pos).unwrap_or_else(|| {
                        let buf_line = text.char_to_line(sel.anchor());
                        let ls = text.line_to_char(buf_line);
                        let le = if buf_line + 1 < text.len_lines() {
                            text.line_to_char(buf_line + 1)
                        } else {
                            text.len_chars()
                        };
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
