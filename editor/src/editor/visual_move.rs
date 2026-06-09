//! Visual-line movement commands (`j`/`k` with soft-wrap).
//!
//! When soft-wrap is active, `j`/`k` move by one display row rather than one
//! buffer line. These commands need access to `wrap_mode`, `tab_width`, and a
//! `FormatScratch` — unavailable in the pure `(&Text, SelectionSet) ->
//! SelectionSet` motion signature — so they live here instead of `ops/motion`.

use editing::selection::Selection;
use engine::pipeline::EngineView;
use super::cursor::format_row_col;
use crate::ops::MotionMode;
use crate::ops::motion::{cmd_move_down, cmd_move_up};
use crate::ops::text_object::{
    apply_nearest_word_result, cmd_select_word_nearest_on_line, nearest_word_on_line,
};
use engine::format::{FormatScratch, format_buffer_line};
use engine::pane::{WhitespaceConfig, WrapMode};
use engine::types::CellContent;

use super::{doc_ops, EditorState};
use crate::editor::error::CommandError;
use super::commands::{focused_buffer_id, focused_format_context};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn find_char_at_display_col(
    scratch: &FormatScratch,
    target_sub_row: usize,
    target_col: u16,
) -> usize {
    let Some(row) = scratch.display_rows.get(target_sub_row) else {
        return 0;
    };
    let graphemes = &scratch.graphemes[row.graphemes.clone()];

    let mut best: Option<(u16, usize)> = None;
    for g in graphemes {
        if g.char_offset == usize::MAX {
            continue;
        }
        if matches!(g.content, CellContent::Empty) {
            continue;
        }
        let dist = target_col.abs_diff(g.col);
        match best {
            None => best = Some((dist, g.char_offset)),
            Some((d, _)) if dist < d => best = Some((dist, g.char_offset)),
            _ => {}
        }
    }

    if best.is_none() {
        for g in graphemes {
            if g.char_offset == usize::MAX {
                continue;
            }
            let dist = target_col.abs_diff(g.col);
            match best {
                None => best = Some((dist, g.char_offset)),
                Some((d, _)) if dist < d => best = Some((dist, g.char_offset)),
                _ => {}
            }
        }
    }

    best.map_or(0, |(_, off)| off)
}

fn visual_move_down_one(
    rope: &ropey::Rope,
    head: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    target_col: u16,
    scratch: &mut FormatScratch,
) -> usize {
    let line = rope.char_to_line(head);
    let (sub_row, _) = format_row_col(rope, line, head, wrap_mode, tab_width, whitespace, scratch);
    let total_sub_rows = scratch.display_rows.len();

    if sub_row + 1 < total_sub_rows {
        find_char_at_display_col(scratch, sub_row + 1, target_col)
    } else {
        let next_line = line + 1;
        if next_line >= rope.len_lines() {
            return head;
        }
        let line_start = rope.line_to_char(next_line);
        if line_start >= rope.len_chars() {
            return head;
        }
        scratch.clear();
        format_buffer_line(rope, next_line, tab_width, whitespace, wrap_mode, &[], scratch);
        find_char_at_display_col(scratch, 0, target_col)
    }
}

fn visual_move_up_one(
    rope: &ropey::Rope,
    head: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    target_col: u16,
    scratch: &mut FormatScratch,
) -> usize {
    let line = rope.char_to_line(head);
    let (sub_row, _) = format_row_col(rope, line, head, wrap_mode, tab_width, whitespace, scratch);

    if sub_row > 0 {
        find_char_at_display_col(scratch, sub_row - 1, target_col)
    } else {
        if line == 0 {
            return head;
        }
        let prev_line = line - 1;
        scratch.clear();
        format_buffer_line(rope, prev_line, tab_width, whitespace, wrap_mode, &[], scratch);
        let last_sub_row = scratch.display_rows.len().saturating_sub(1);
        find_char_at_display_col(scratch, last_sub_row, target_col)
    }
}

/// Shared core for the four visual-line movement EditorCmds.
///
/// When wrapping is off every buffer line is exactly one display row, so we
/// fall back to the pure buffer-line motions to avoid any overhead.
pub(super) fn apply_visual_vertical(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    down: bool,
    mode: MotionMode,
) {
    let (wrap_mode, tab_width, whitespace) = focused_format_context(state, view);

    if !wrap_mode.is_wrapping() {
        let focused = state.focused_pane_id;
        let buf = focused_buffer_id(state, view);
        match down {
            true => doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
                cmd_move_down(b, s, count, mode)
            }),
            false => doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
                cmd_move_up(b, s, count, mode)
            }),
        }
        return;
    }

    let focused = state.focused_pane_id;
    let buf_id = focused_buffer_id(state, view);
    let scratch = &mut state.motion_format_scratch;
    let target_cols = &mut state.visual_move_target_cols;
    target_cols.clear();

    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf_id, |text, sels| {
        let rope = text.rope();

        target_cols.extend(sels.iter_sorted().map(|sel| {
            if let Some(col) = sel.horiz() {
                col as u16
            } else {
                let line = rope.char_to_line(sel.head());
                let (_, col) = format_row_col(rope, line, sel.head(), &wrap_mode, tab_width, &whitespace, scratch);
                col as u16
            }
        }));

        let cols: &[u16] = target_cols;
        let mut col_iter = cols.iter();
        sels.map(|sel| {
            let &target_col = col_iter.next().unwrap();
            let mut head = sel.head();
            for _ in 0..count {
                head = if down {
                    visual_move_down_one(rope, head, &wrap_mode, tab_width, &whitespace, target_col, scratch)
                } else {
                    visual_move_up_one(rope, head, &wrap_mode, tab_width, &whitespace, target_col, scratch)
                };
            }
            let anchor = if mode == MotionMode::Extend { sel.anchor() } else { head };
            Selection::with_horiz(anchor, head, target_col as u32)
        })
    });
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
    apply_visual_vertical(state, view, count, true, mode);
    Ok(())
}

pub(super) fn cmd_visual_move_up(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    apply_visual_vertical(state, view, count, false, mode);
    Ok(())
}

fn sub_row_char_bounds(
    scratch: &FormatScratch,
    sub_row: usize,
    buf_line: usize,
    rope: &ropey::Rope,
) -> Option<(usize, usize)> {
    let row = scratch.display_rows.get(sub_row)?;
    let char_start = scratch.graphemes[row.graphemes.clone()]
        .iter()
        .filter(|g| g.char_offset != usize::MAX)
        .map(|g| g.char_offset)
        .min()?;

    let next_buf_line_start = rope.line_to_char(buf_line + 1);

    let char_end_excl = scratch
        .display_rows
        .get(sub_row + 1)
        .and_then(|next_row| {
            scratch.graphemes[next_row.graphemes.clone()]
                .iter()
                .filter(|g| g.char_offset != usize::MAX)
                .map(|g| g.char_offset)
                .min()
        })
        .unwrap_or(next_buf_line_start);

    Some((char_start, char_end_excl))
}

pub(super) fn cmd_visual_select_word_nearest_on_line(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let (wrap_mode, tab_width, whitespace) = focused_format_context(state, view);

    if !wrap_mode.is_wrapping() {
        let focused = state.focused_pane_id;
        let buf_id = focused_buffer_id(state, view);
        doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf_id, |buf, sels| {
            cmd_select_word_nearest_on_line(buf, sels, mode)
        });
        return Ok(());
    }

    let focused = state.focused_pane_id;
    let buf_id = focused_buffer_id(state, view);
    let scratch = &mut state.motion_format_scratch;

    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf_id, |text, sels| {
        let rope = text.rope();
        let new_sels = sels.map(|sel| {
            let buf_line = text.char_to_line(sel.anchor());
            let (sub_row, _) =
                format_row_col(rope, buf_line, sel.anchor(), &wrap_mode, tab_width, &whitespace, scratch);

            let (line_start, line_end_excl) =
                sub_row_char_bounds(scratch, sub_row, buf_line, rope).unwrap_or_else(|| {
                    let ls = text.line_to_char(buf_line);
                    let le = if buf_line + 1 < text.len_lines() {
                        text.line_to_char(buf_line + 1)
                    } else {
                        text.len_chars()
                    };
                    (ls, le)
                });

            let found = nearest_word_on_line(text, sel.anchor(), line_start, line_end_excl);
            apply_nearest_word_result(sel, found, mode)
        });
        new_sels.debug_assert_valid(text);
        new_sels
    });

    Ok(())
}
