//! Visual-line movement commands (`j`/`k` with soft-wrap).
//!
//! When soft-wrap is active, `j`/`k` move by one display row rather than one
//! buffer line. These commands need access to `wrap_mode`, `tab_width`, and a
//! `FormatScratch` — unavailable in the pure `(&Text, SelectionSet) ->
//! SelectionSet` motion signature — so they live here instead of `ops/motion`.

use super::cursor::format_row_col;
use crate::ops::MotionMode;
use crate::ops::motion::{cmd_move_down, cmd_move_up};
use crate::ops::text_object::{
    apply_nearest_word_result, cmd_select_word_nearest_on_line, nearest_word_on_line,
};
use hume_editing::selection::Selection;
use hume_engine::format::{FormatScratch, display_rows_for_line, format_buffer_line};
use hume_engine::pane::{WhitespaceConfig, WrapMode};
use hume_engine::pipeline::EngineView;
use hume_engine::providers::ProviderSet;
use hume_engine::types::CellContent;

use super::commands::{apply_focused_motion, focused_buffer_id, focused_format_context};
use super::{EditorState, doc_ops};
use crate::editor::error::CommandError;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Find the char offset of the grapheme in `target_sub_row` closest to
/// `target_col` display columns.
///
/// Prefers real content graphemes over the end-of-line sentinel (the `Empty`
/// grapheme emitted at the `\n` position). The sentinel is only used as a
/// fallback for empty lines where it is the only grapheme in the row.
/// Virtual fill cells (`char_offset == usize::MAX`) are always skipped.
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
    let mut best: Option<(u16, usize)> = None;
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
            None,
            &[],
            scratch,
        );
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
        format_buffer_line(
            rope,
            prev_line,
            tab_width,
            whitespace,
            wrap_mode,
            None,
            &[],
            scratch,
        );
        let last_sub_row = scratch.display_rows.len().saturating_sub(1);
        find_char_at_display_col(scratch, last_sub_row, target_col)
    }
}

/// A row within a buffer line's visual block (`before`/content/`after` —
/// see `ViewportState::top_row_offset`'s doc), used by `screen_move_vertical`
/// to walk display rows one at a time without resolving a char offset for
/// every row crossed (only `Content` rows are valid cursor positions).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Before,
    Content,
    After,
}

#[derive(Clone, Copy)]
struct BlockPos {
    line: usize,
    kind: RowKind,
    row: usize,
}

/// Advance (or retreat) `pos` by exactly one display row, crossing line
/// boundaries as needed. Returns `None` at the buffer's start/end — the
/// caller clamps to the last position reached.
#[allow(clippy::too_many_arguments)]
fn step_block_row(
    pos: BlockPos,
    down: bool,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    providers: &ProviderSet,
    content_width: u16,
    scratch: &mut FormatScratch,
    last_line: usize,
) -> Option<BlockPos> {
    let breakdown = display_rows_for_line(
        rope,
        pos.line,
        tab_width,
        whitespace,
        wrap_mode,
        providers,
        content_width,
        scratch,
    );
    if down {
        let next_in_kind = |kind: RowKind, count: usize| {
            (pos.row + 1 < count).then_some(BlockPos {
                row: pos.row + 1,
                kind,
                ..pos
            })
        };
        match pos.kind {
            RowKind::Before => next_in_kind(RowKind::Before, breakdown.before).or(Some(BlockPos {
                kind: RowKind::Content,
                row: 0,
                ..pos
            })),
            RowKind::Content => next_in_kind(RowKind::Content, breakdown.content)
                .or_else(|| {
                    (breakdown.after > 0).then_some(BlockPos {
                        kind: RowKind::After,
                        row: 0,
                        ..pos
                    })
                })
                .or_else(|| {
                    cross_line_down(
                        pos.line,
                        last_line,
                        rope,
                        wrap_mode,
                        tab_width,
                        whitespace,
                        providers,
                        content_width,
                        scratch,
                    )
                }),
            RowKind::After => next_in_kind(RowKind::After, breakdown.after).or_else(|| {
                cross_line_down(
                    pos.line,
                    last_line,
                    rope,
                    wrap_mode,
                    tab_width,
                    whitespace,
                    providers,
                    content_width,
                    scratch,
                )
            }),
        }
    } else {
        // `.then(||...)`, not `.then_some(...)` — the row subtractions below
        // must not be eagerly evaluated when the guard is false (Rust always
        // evaluates a plain argument before the call, so `then_some` would
        // still underflow `row - 1` at `row == 0` even though it discards
        // the result).
        let prev_in_kind = |kind: RowKind| {
            (pos.row > 0).then(|| BlockPos {
                row: pos.row - 1,
                kind,
                ..pos
            })
        };
        match pos.kind {
            RowKind::After => prev_in_kind(RowKind::After).or(Some(BlockPos {
                kind: RowKind::Content,
                row: breakdown.content.saturating_sub(1),
                ..pos
            })),
            RowKind::Content => prev_in_kind(RowKind::Content)
                .or_else(|| {
                    (breakdown.before > 0).then(|| BlockPos {
                        kind: RowKind::Before,
                        row: breakdown.before - 1,
                        ..pos
                    })
                })
                .or_else(|| {
                    cross_line_up(
                        pos.line,
                        rope,
                        wrap_mode,
                        tab_width,
                        whitespace,
                        providers,
                        content_width,
                        scratch,
                    )
                }),
            RowKind::Before => prev_in_kind(RowKind::Before).or_else(|| {
                cross_line_up(
                    pos.line,
                    rope,
                    wrap_mode,
                    tab_width,
                    whitespace,
                    providers,
                    content_width,
                    scratch,
                )
            }),
        }
    }
}

/// Cross from `line` into `line + 1`'s first row (its `before` block if it
/// has one, else its first content row — content is never empty). Mirrors
/// `cross_line_up`: must resolve the target line's actual starting kind
/// here, not just assume `Before, row: 0` and rely on the next
/// `step_block_row` call to fall through to `Content` — that would cost an
/// extra budget unit crossing a line with zero `before` rows.
#[allow(clippy::too_many_arguments)]
fn cross_line_down(
    line: usize,
    last_line: usize,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    providers: &ProviderSet,
    content_width: u16,
    scratch: &mut FormatScratch,
) -> Option<BlockPos> {
    if line >= last_line {
        return None;
    }
    let next_line = line + 1;
    let breakdown = display_rows_for_line(
        rope,
        next_line,
        tab_width,
        whitespace,
        wrap_mode,
        providers,
        content_width,
        scratch,
    );
    Some(if breakdown.before > 0 {
        BlockPos {
            line: next_line,
            kind: RowKind::Before,
            row: 0,
        }
    } else {
        BlockPos {
            line: next_line,
            kind: RowKind::Content,
            row: 0,
        }
    })
}

/// Cross from `line` into `line - 1`'s last row (its `after` block if it
/// has one, else its last content row).
#[allow(clippy::too_many_arguments)]
fn cross_line_up(
    line: usize,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    providers: &ProviderSet,
    content_width: u16,
    scratch: &mut FormatScratch,
) -> Option<BlockPos> {
    if line == 0 {
        return None;
    }
    let prev_line = line - 1;
    let breakdown = display_rows_for_line(
        rope,
        prev_line,
        tab_width,
        whitespace,
        wrap_mode,
        providers,
        content_width,
        scratch,
    );
    Some(if breakdown.after > 0 {
        BlockPos {
            line: prev_line,
            kind: RowKind::After,
            row: breakdown.after - 1,
        }
    } else {
        BlockPos {
            line: prev_line,
            kind: RowKind::Content,
            row: breakdown.content.saturating_sub(1),
        }
    })
}

/// Move `head` down (or up) by `count` **display** rows — virtual `before`/
/// `after` rows (from any `VirtualLineSource`) consume a unit of the budget
/// but are never a valid landing spot. Unlike `visual_move_down_one`/`up_one`
/// (per-press `j`/`k` semantics, where a virtual row is free and never costs
/// a keystroke), this is for callers where `count` already IS a display-row
/// measurement of something else — the mouse wheel's or page-scroll's own
/// viewport delta — and must track it 1:1, including virtual rows, so the
/// cursor stays at roughly the same relative screen row the viewport just
/// moved to. Lands on the last real content row reached when the budget
/// runs out (or the buffer's start/end, clamped, if it runs out first).
#[allow(clippy::too_many_arguments)]
fn screen_move_vertical(
    rope: &ropey::Rope,
    head: usize,
    down: bool,
    count: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    target_col: u16,
    providers: &ProviderSet,
    content_width: u16,
    scratch: &mut FormatScratch,
) -> usize {
    let last_line = rope.len_lines().saturating_sub(2);
    let line = rope.char_to_line(head);
    let (sub, _) = format_row_col(rope, line, head, wrap_mode, tab_width, whitespace, scratch);

    let mut pos = BlockPos {
        line,
        kind: RowKind::Content,
        row: sub,
    };
    let mut last_content = pos;
    for _ in 0..count {
        let Some(next) = step_block_row(
            pos,
            down,
            rope,
            wrap_mode,
            tab_width,
            whitespace,
            providers,
            content_width,
            scratch,
            last_line,
        ) else {
            break; // buffer start/end — clamp to the last position reached
        };
        pos = next;
        if pos.kind == RowKind::Content {
            last_content = pos;
        }
    }

    scratch.clear();
    format_buffer_line(
        rope,
        last_content.line,
        tab_width,
        whitespace,
        wrap_mode,
        None,
        &[],
        scratch,
    );
    find_char_at_display_col(scratch, last_content.row, target_col)
}

/// How `apply_visual_vertical`'s `count` should be interpreted.
pub(super) enum VerticalUnit {
    /// `count` buffer lines — `j`/`k` with an explicit numeric prefix
    /// (matches relative-line-number gutters even while wrapping).
    BufferLine,
    /// `count` real content rows; virtual rows are free (never cost a
    /// keystroke, never a landing spot) — plain `j`/`k` with no explicit
    /// count.
    ContentRow,
    /// `count` display rows, virtual rows included — mouse wheel and
    /// page/half-page scroll. See `screen_move_vertical`'s doc.
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
    let (wrap_mode, tab_width, whitespace) = focused_format_context(state, view);

    // No-wrap content rows are buffer lines exactly (no sub-row stepping
    // possible), so `ContentRow` degenerates to the same buffer-line motion
    // as `BufferLine` there — cheaper, and avoids a wasted `display_rows_for_line`
    // walk when there's nothing to walk over.
    let use_buffer_line_motion = matches!(unit, VerticalUnit::BufferLine)
        || (matches!(unit, VerticalUnit::ContentRow) && !wrap_mode.is_wrapping());
    if use_buffer_line_motion {
        let motion = if down { cmd_move_down } else { cmd_move_up };
        apply_focused_motion(state, view, |b, s| motion(b, s, count, mode));
        return;
    }
    // Only `ScreenRow`, or `ContentRow` while wrapping, reach here.
    let use_screen_row = matches!(unit, VerticalUnit::ScreenRow);

    let focused = state.focused_pane_id;
    let buf_id = focused_buffer_id(state, view);
    // `ScreenRow` needs the pane's providers/content_width to see virtual
    // lines — fetched here (from `view`, disjoint from the `state` fields
    // the closure below borrows) rather than threaded through every caller.
    let pane = &view.panes[focused];
    let content_width = pane.content_width(state.buffers.get(buf_id).text().len_lines());
    let providers = &pane.providers;
    let scratch = &mut state.motion_format_scratch;
    let target_cols = &mut state.visual_move_target_cols;
    target_cols.clear();

    // Not `apply_focused_motion`: the closure below also captures `scratch`
    // and `target_cols`, disjoint fields of `state` that must be borrowed
    // separately from `state.buffers`/`state.panes.state` here.
    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        focused,
        buf_id,
        |text, sels| {
            let rope = text.rope();

            // Pass 1: resolve each selection's sticky display column from sel.horiz,
            // computing it fresh on the first j/k press (when horiz is None).
            target_cols.extend(sels.iter_sorted().map(|sel| {
                if let Some(col) = sel.horiz() {
                    col as u16
                } else {
                    let line = rope.char_to_line(sel.head());
                    let (_, col) = format_row_col(
                        rope,
                        line,
                        sel.head(),
                        &wrap_mode,
                        tab_width,
                        &whitespace,
                        scratch,
                    );
                    col as u16
                }
            }));

            // Pass 2: move each selection by `count` display rows, preserving the
            // sticky column in sel.horiz so consecutive j/k presses reuse it.
            let cols: &[u16] = target_cols;
            let mut col_iter = cols.iter();
            sels.map(|sel| {
                let &target_col = col_iter.next().unwrap();
                let mut head = sel.head();
                let head = if use_screen_row {
                    screen_move_vertical(
                        rope,
                        head,
                        down,
                        count,
                        &wrap_mode,
                        tab_width,
                        &whitespace,
                        target_col,
                        providers,
                        content_width,
                        scratch,
                    )
                } else {
                    for _ in 0..count {
                        head = if down {
                            visual_move_down_one(
                                rope,
                                head,
                                &wrap_mode,
                                tab_width,
                                &whitespace,
                                target_col,
                                scratch,
                            )
                        } else {
                            visual_move_up_one(
                                rope,
                                head,
                                &wrap_mode,
                                tab_width,
                                &whitespace,
                                target_col,
                                scratch,
                            )
                        };
                    }
                    head
                };
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

    // HUME buffers always end with '\n', so buf_line + 1 is always a valid line index.
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

/// Wrap-aware variant of `select-word-nearest-on-line`.
///
/// When wrap is active, scopes the nearest-word search to the head's current
/// visual sub-row rather than the full buffer line. This prevents the search
/// from finding words that live on an adjacent visual row when the head lands
/// on leading whitespace near a wrap boundary — the failure mode that causes
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
    let (wrap_mode, tab_width, whitespace) = focused_format_context(state, view);
    let buf_id = focused_buffer_id(state, view);
    let around = state
        .buffers
        .get(buf_id)
        .overrides
        .word_selects_whitespace(&state.settings);

    if !wrap_mode.is_wrapping() {
        apply_focused_motion(state, view, |buf, sels| {
            cmd_select_word_nearest_on_line(buf, sels, 0, mode, around)
        });
        return Ok(());
    }

    let focused = state.focused_pane_id;
    let scratch = &mut state.motion_format_scratch;

    // Not `apply_focused_motion`: the closure below also captures `scratch`.
    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        focused,
        buf_id,
        |text, sels| {
            let rope = text.rope();
            let new_sels = sels.map(|sel| {
                let buf_line = text.char_to_line(sel.anchor());
                let (sub_row, _) = format_row_col(
                    rope,
                    buf_line,
                    sel.anchor(),
                    &wrap_mode,
                    tab_width,
                    &whitespace,
                    scratch,
                );

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
