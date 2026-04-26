//! Visual-line movement commands (`j`/`k` with soft-wrap).
//!
//! When soft-wrap is active, `j`/`k` move by one display row rather than one
//! buffer line. These commands need access to `wrap_mode`, `tab_width`, and a
//! `FormatScratch` — unavailable in the pure `(&Text, SelectionSet) ->
//! SelectionSet` motion signature — so they live here instead of `ops/motion`.

use crate::core::selection::Selection;
use crate::cursor::format_row_col;
use crate::ops::MotionMode;
use crate::ops::motion::{cmd_move_down, cmd_move_up};
use engine::format::{FormatScratch, format_buffer_line};
use engine::pane::{WhitespaceConfig, WrapMode};
use engine::types::CellContent;

use super::Editor;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Find the char offset of the grapheme in `target_sub_row` closest to
/// `target_col` display columns, given an already-formatted scratch buffer.
///
/// Prefers real content graphemes over the end-of-line sentinel (the `Empty`
/// grapheme emitted at the `\n` position). The sentinel is only used as a
/// fallback on truly empty lines where it is the only grapheme. Virtual fill
/// cells (`char_offset == usize::MAX`) are always skipped.
///
/// Returns `0` if the row has no graphemes at all.
fn find_char_at_display_col(
    scratch: &FormatScratch,
    target_sub_row: usize,
    target_col: u16,
) -> usize {
    let Some(row) = scratch.display_rows.get(target_sub_row) else {
        return 0;
    };
    let graphemes = &scratch.graphemes[row.graphemes.clone()];

    // First pass: real content graphemes only (skip Empty sentinel and virtual cells).
    let mut best: Option<(u16, usize)> = None; // (distance, char_offset)
    for g in graphemes {
        if g.char_offset == usize::MAX {
            continue;
        } // virtual/fill cell
        if matches!(g.content, CellContent::Empty) {
            continue;
        } // eol sentinel
        let dist = target_col.abs_diff(g.col);
        match best {
            None => best = Some((dist, g.char_offset)),
            Some((d, _)) if dist < d => best = Some((dist, g.char_offset)),
            _ => {}
        }
    }

    // Fallback: include Empty sentinel (empty lines where it is the only grapheme).
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

/// Advance `head` by one display row downward using the given wrap config.
///
/// Returns the new char offset. Stays put when already on the last display row
/// of the last buffer line.
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

    // format_row_col clears scratch and formats the current buffer line.
    let (sub_row, _) = format_row_col(rope, line, head, wrap_mode, tab_width, whitespace, scratch);
    let total_sub_rows = scratch.display_rows.len();

    if sub_row + 1 < total_sub_rows {
        // Stay on the same buffer line — just advance one display sub-row.
        // scratch already holds the formatted current line.
        find_char_at_display_col(scratch, sub_row + 1, target_col)
    } else {
        // Cross to the next buffer line.
        let next_line = line + 1;
        if next_line >= rope.len_lines() {
            return head;
        }
        let line_start = rope.line_to_char(next_line);
        // Guard against the phantom trailing line (structural trailing \n).
        if line_start >= rope.len_chars() {
            return head;
        }
        scratch.clear();
        format_buffer_line(
            rope,
            next_line,
            tab_width,
            whitespace,
            wrap_mode,
            &[],
            scratch,
        );
        find_char_at_display_col(scratch, 0, target_col)
    }
}

/// Retreat `head` by one display row upward using the given wrap config.
///
/// Returns the new char offset. Stays put when already on the first display
/// row of the first buffer line.
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
        // Stay on the same buffer line — retreat one display sub-row.
        find_char_at_display_col(scratch, sub_row - 1, target_col)
    } else {
        // Cross to the previous buffer line.
        if line == 0 {
            return head;
        }
        let prev_line = line - 1;
        scratch.clear();
        format_buffer_line(
            rope,
            prev_line,
            tab_width,
            whitespace,
            wrap_mode,
            &[],
            scratch,
        );
        let last_sub_row = scratch.display_rows.len().saturating_sub(1);
        find_char_at_display_col(scratch, last_sub_row, target_col)
    }
}

/// Shared core for the four visual-line movement EditorCmds.
///
/// When wrapping is off every buffer line is exactly one display row, so we
/// fall back to the pure buffer-line motions to avoid any overhead.
fn apply_visual_vertical(ed: &mut Editor, count: usize, down: bool, mode: MotionMode) {
    let raw_wrap = ed.doc().overrides.wrap_mode(&ed.settings);
    let tab_width = ed.doc().overrides.tab_width(&ed.settings);
    let whitespace = ed.doc().overrides.whitespace(&ed.settings);

    if !raw_wrap.is_wrapping() {
        // No wrapping — fall back to buffer-line movement.
        // Selection.horiz is None on collapsed/new selections by default, so no explicit clear needed.
        match down {
            true => ed.apply_motion(|b, s| cmd_move_down(b, s, count, mode)),
            false => ed.apply_motion(|b, s| cmd_move_up(b, s, count, mode)),
        }
        return;
    }

    let wrap_mode = {
        let pane = &ed.engine_view.panes[ed.focused_pane_id];
        raw_wrap.resolve(pane.content_width(ed.doc().text().len_lines()))
    };

    let rope = ed.doc().text().rope().clone();
    let sels = ed.current_selections().clone();

    // Pass 1: resolve each selection's sticky display column from sel.horiz,
    // computing it fresh on the first j/k press (when horiz is None).
    let target_cols: Vec<u16> = sels
        .iter_sorted()
        .map(|sel| {
            if let Some(col) = sel.horiz {
                col as u16
            } else {
                let line = rope.char_to_line(sel.head);
                let (_, col) = format_row_col(
                    &rope,
                    line,
                    sel.head,
                    &wrap_mode,
                    tab_width,
                    &whitespace,
                    &mut ed.motion_format_scratch,
                );
                col as u16
            }
        })
        .collect();

    // Pass 2: move each selection by `count` display rows, preserving the
    // sticky column in sel.horiz so consecutive j/k presses reuse it.
    let mut col_iter = target_cols.iter();
    let scratch = &mut ed.motion_format_scratch;
    let new_sels = sels.map_and_merge(|sel| {
        let &target_col = col_iter.next().unwrap();
        let mut head = sel.head;
        for _ in 0..count {
            head = if down {
                visual_move_down_one(
                    &rope,
                    head,
                    &wrap_mode,
                    tab_width,
                    &whitespace,
                    target_col,
                    scratch,
                )
            } else {
                visual_move_up_one(
                    &rope,
                    head,
                    &wrap_mode,
                    tab_width,
                    &whitespace,
                    target_col,
                    scratch,
                )
            };
        }
        let anchor = if mode == MotionMode::Extend {
            sel.anchor
        } else {
            head
        };
        Selection::with_horiz(anchor, head, target_col as u32)
    });

    ed.set_current_selections(new_sels);
}

// ---------------------------------------------------------------------------
// Public commands
// ---------------------------------------------------------------------------

pub(super) fn cmd_visual_move_down(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), crate::core::error::CommandError> {
    apply_visual_vertical(ed, count, true, mode);
    Ok(())
}

pub(super) fn cmd_visual_move_up(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), crate::core::error::CommandError> {
    apply_visual_vertical(ed, count, false, mode);
    Ok(())
}
