//! Vertical commands that need a `RowMap` — unavailable in the pure
//! `(&BufferText, SelectionSet) -> SelectionSet` motion signature — so they
//! live here instead of `hume-ops`'s `motion`/`selection_cmd` modules.
//!
//! Two families: `j`/`k` movement, which under soft-wrap moves by one display
//! row rather than one buffer line; and `copy-selection-on-{next,prev}-line`
//! (`C`), which needs the same display-column authority to land a duplicated
//! selection under a tab or wide grapheme without wrap in play at all.

use hume_editing::selection::{DisplayColOrigin, Selection, SelectionSet, StickyDisplayCol};
use hume_editing::text::BufferText;
use hume_editing::word::WordChars;
use hume_engine::pipeline::EngineView;
use hume_engine::rows::{BlockSlot, DisplayColTarget, RowMap};
use hume_ops::text_object::{
    apply_nearest_word_result, cmd_select_word_nearest_on_line, nearest_word_on_line,
};
use hume_ops::{MotionMode, WordCtx};

use super::commands::{
    apply_focused_motion, effective_wrap_mode, focused_buffer_id, pane_row_map, word_chars_owned,
};
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
        let is_content = matches!(rm.slot(pos), BlockSlot::Content(_));
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
    text: &BufferText,
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
    // time — see `DisplayColOrigin`. Resolved once per call, and before the
    // row map takes the pane mutably.
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
    let buffer_tag = state.buffer_tag(buf_id);
    let target_display_cols = &mut state.visual_move_target_display_cols;
    target_display_cols.clear();
    let mut rm = pane_row_map(
        state.buffers.get(buf_id),
        &state.settings,
        &mut view.panes[focused],
        buffer_tag,
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
// Vertical selection copy
// ---------------------------------------------------------------------------

/// Duplicate each selection onto each of the `count` lines below it (`down:
/// true`) or above it (`false`) and add them to the selection set, landing
/// each copy's anchor and head on the *display* column of the original —
/// needs a `RowMap`, so, like the
/// visual-line motions above, this lives here rather than in `hume-ops`'s
/// pure `(&BufferText, SelectionSet) -> SelectionSet` signature (see this
/// module's doc comment).
///
/// `DisplayColTarget::NearestContent` reproduces the clamp rule a plain
/// column placement already needs: stick to the last real character on a
/// short target line, land on `\n` only when that line is empty.
///
/// Each successive copy is offset by the *selection's own line span* (1 for
/// a single-line selection, more for one spanning several buffer lines), not
/// by one line — stepping by one would leave a multi-line copy overlapping
/// the selection it came from, which `SelectionSet::from_vec`'s merge would
/// then fold back into a single, grown selection instead of a duplicate.
/// Clamped to how many whole spans fit before the buffer's last real content
/// line (or its start, going up) — a `count` larger than that just lands the
/// last copy that fits, it doesn't wrap, error, or land partially past the
/// end.
///
/// The primary advances to the furthest copy of the original primary. Every
/// copy's column is re-derived from the *original* selection, not the
/// previous copy — so this is not equivalent to `count` separate presses of
/// the count-1 command, which would re-clamp against each intermediate line
/// in turn. Re-deriving from the original means a single short line in the
/// middle of the run only clamps that one copy, instead of collapsing every
/// copy after it to that line's column. If no copy was added (last-line edge
/// case) the primary stays on the original.
fn copy_selection_vertically(
    rm: &mut RowMap<'_>,
    text: &BufferText,
    sels: SelectionSet,
    down: bool,
    count: usize,
) -> SelectionSet {
    let direction: isize = if down { 1 } else { -1 };
    let primary_idx = sels.primary_index();
    // Collect originals into `all_sels`. Copies are appended below.
    let mut all_sels: Vec<Selection> = sels.iter_sorted().copied().collect();
    let original_len = all_sels.len();
    // Index in `all_sels` for the furthest copy of the old primary, if one was added.
    let mut primary_copy_idx: Option<usize> = None;

    for i in 0..original_len {
        let sel = all_sels[i];
        let anchor_line = text.char_to_line(sel.anchor()) as isize;
        let head_line = text.char_to_line(sel.head()) as isize;

        // The outermost line in the copy direction determines the offset target.
        let outer_line = if down {
            anchor_line.max(head_line) // bottommost for "down"
        } else {
            anchor_line.min(head_line) // topmost for "up"
        };
        let span = (anchor_line - head_line).unsigned_abs() as isize + 1;

        // Both endpoints' display columns are loop-invariant — the original
        // selection never changes across copies — so compute them once
        // instead of re-deriving on every iteration.
        let anchor_display_col = rm.line_display_col(sel.anchor());
        let head_display_col = rm.line_display_col(sel.head());

        // How many whole `span`-line steps fit between the selection and the
        // buffer's edge in `direction`, floor-divided — the last step that
        // still lands fully on real content. Kept in `usize` throughout: a
        // `count` of `usize::MAX` must clamp here without ever appearing in
        // an `isize` computation, which `available.min(count)` guarantees.
        let available = if down {
            (text.last_content_line() as isize - outer_line).max(0) / span
        } else {
            outer_line / span
        } as usize;
        let steps = available.min(count);

        for step in 1..=steps {
            let delta = step as isize * span * direction;
            let new_anchor = rm.char_at_line_display_col(
                (anchor_line + delta) as usize,
                anchor_display_col,
                DisplayColTarget::NearestContent,
            );
            let new_head = rm.char_at_line_display_col(
                (head_line + delta) as usize,
                head_display_col,
                DisplayColTarget::NearestContent,
            );

            let new_sel = Selection::new(new_anchor, new_head);

            if i == primary_idx {
                primary_copy_idx = Some(all_sels.len());
            }
            all_sels.push(new_sel);
        }
    }

    let desired_primary = primary_copy_idx.unwrap_or(primary_idx);
    let new_set = SelectionSet::from_vec(all_sels, desired_primary);
    new_set.debug_assert_valid(text);
    new_set
}

/// Shared body of [`cmd_copy_selection_on_next_line`]/[`cmd_copy_selection_on_prev_line`].
///
/// Builds the `RowMap` [`copy_selection_vertically`] needs before entering
/// `apply_doc_motion`. One `RowMap` line-format per selection per target
/// line — for a lone cursor this is the same per-line cost `9j`/`9k` already
/// pay for the same reason (a rope-only column can't see tabs or the
/// decoration layer); with `count` copies of several selections the cost
/// multiplies, since `RowMap` caches only the one line it last formatted.
fn copy_selection_on_line(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    down: bool,
) {
    let focused = state.focused_pane_id;
    let buf_id = focused_buffer_id(state, view);
    let buffer_tag = state.buffer_tag(buf_id);
    let mut rm = pane_row_map(
        state.buffers.get(buf_id),
        &state.settings,
        &mut view.panes[focused],
        buffer_tag,
    );

    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        focused,
        buf_id,
        |text, sels| copy_selection_vertically(&mut rm, text, sels, down, count),
    );
}

// ---------------------------------------------------------------------------
// Public commands
// ---------------------------------------------------------------------------

/// Shared body of [`cmd_visual_move_down`]/[`cmd_visual_move_up`] — the two
/// differ only in `down`, so they delegate here rather than each carrying
/// their own copy of the count-unit decision.
fn visual_move_vertical(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    down: bool,
    mode: MotionMode,
) {
    // A count typed by the user (e.g. `9j`) means "9 buffer lines" — matching
    // relative-line-number gutters — even when soft-wrap is on.
    let unit = if state.explicit_count {
        VerticalUnit::BufferLine
    } else {
        VerticalUnit::ContentRow
    };
    apply_visual_vertical(state, view, count, down, mode, unit);
}

pub(super) fn cmd_visual_move_down(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    visual_move_vertical(state, view, count, true, mode);
    Ok(())
}

pub(super) fn cmd_visual_move_up(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    visual_move_vertical(state, view, count, false, mode);
    Ok(())
}

/// Duplicate each selection on the line below.
pub(super) fn cmd_copy_selection_on_next_line(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    copy_selection_on_line(state, view, count, true);
    Ok(())
}

/// Duplicate each selection on the line above.
pub(super) fn cmd_copy_selection_on_prev_line(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    copy_selection_on_line(state, view, count, false);
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
    // Owned, not borrowed: the no-wrap branch below calls
    // `apply_focused_motion(state, ...)`, which takes `&mut EditorState` as
    // one opaque argument — a live borrow into `state.buffers`/`state.settings`
    // (what a borrowed `chars` would be) can't survive across that call.
    let word_chars = word_chars_owned(doc, &state.settings);
    let chars = WordChars::new(&word_chars);
    let ctx = WordCtx {
        mode,
        around,
        chars,
    };

    if !effective_wrap_mode(doc, &state.settings, &view.panes[state.focused_pane_id]).is_wrapping()
    {
        apply_focused_motion(state, view, |text, sels| {
            cmd_select_word_nearest_on_line(text, sels, 0, ctx)
        });
        return Ok(());
    }

    let focused = state.focused_pane_id;
    let buffer_tag = state.buffer_tag(buf_id);
    let mut rm = pane_row_map(
        state.buffers.get(buf_id),
        &state.settings,
        &mut view.panes[focused],
        buffer_tag,
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

                let found = nearest_word_on_line(
                    text,
                    sel.anchor(),
                    line_start,
                    line_end_excl,
                    around,
                    chars,
                );
                apply_nearest_word_result(sel, found, mode)
            });
            new_sels.debug_assert_valid(text);
            new_sels
        },
    );

    Ok(())
}
