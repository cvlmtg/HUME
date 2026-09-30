//! Vertical commands that need a `DisplayLineMap` (unavailable in the pure
//! `(EditState, usize, MotionMode) -> EditState` motion signature), so they
//! live here instead of `hume-ops`'s `motion`/`selection_cmd` modules.
//!
//! Two families: `j`/`k` movement, which under soft-wrap moves by one display
//! line rather than one buffer line; and `copy-selection-on-{next,prev}-line`
//! (`C`), which needs the same display-column authority to land a duplicated
//! selection under a tab or wide grapheme without wrap in play at all.

use hume_editing::lines::{line_break, line_range};
use hume_editing::selection::{Selection, StickyDisplayCol};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::WordChars;
use hume_engine::display_lines::{DisplayColTarget, DisplayLineMap};
use hume_engine::pipeline::{EngineView, PaneId};
use hume_ops::text_object::{
    apply_nearest_word_result, cmd_select_word_nearest_on_line, nearest_word_on_line,
};
use hume_ops::{MotionMode, WordCtx};
use hume_rope::cluster::ClusterStart;
use hume_rope::column::{BufferLineCol, DisplayLineCol};

use super::commands::{
    CommandPane, apply_pane_motion, effective_wrap_mode, pane_display_lines, word_chars_owned,
};
use super::{EditorState, doc_ops};
use crate::editor::error::CommandError;

// ---------------------------------------------------------------------------
// Vertical movement
// ---------------------------------------------------------------------------

/// Move `head` by `count` content display lines, landing on the last one
/// reached (or staying put if the document's edge came first). Virtual
/// display lines are walked through but count against neither the budget nor
/// as a landing spot, so a virtual-line decoration source's display lines
/// never swallow a `j`/`k` keystroke.
///
/// The mouse wheel and page/half-page scroll carry their cursor through
/// `hume_engine::display_lines::carry` instead: a different question (track
/// the view's own display-line delta 1:1, virtual lines included) with a
/// different contract (park rather than land, when nothing fits), so it lives
/// in the engine next to `Viewport::scroll_by` rather than as a second mode
/// of this function.
fn move_vertical(
    dlm: &mut DisplayLineMap<'_>,
    head: ClusterStart,
    down: bool,
    count: usize,
    target_display_col: DisplayLineCol,
) -> ClusterStart {
    let start = dlm.locate_display_line(head);
    let mut pos = start;
    let mut last_content = start;
    let mut remaining = count;

    while remaining > 0 {
        let Some(next) = (if down { dlm.next(pos) } else { dlm.prev(pos) }) else {
            break; // document start/end: clamp to the last content display line reached
        };
        pos = next;
        if dlm.slot(pos).is_content() {
            last_content = pos;
            remaining -= 1;
        }
    }

    if last_content == start {
        // No content display line in this direction: the document's own
        // start/end broke the walk above before one could be found. Leave
        // the head exactly where it was rather than snapping it to
        // `target_display_col`.
        return head;
    }
    dlm.char_at(
        last_content,
        target_display_col,
        DisplayColTarget::NearestContent,
    )
}

/// Move `head` by `count` buffer lines, landing on the target line's own
/// line-relative display column (`DisplayLineMap::char_at_buffer_line_col`).
///
/// Distinct from `move_vertical`: a numeric-prefixed vertical move (`9j`) is a
/// direct line-index jump matching relative-line-number gutters, not a
/// display-line walk. Virtual and wrap display lines are both irrelevant to it.
fn move_buffer_line(
    dlm: &mut DisplayLineMap<'_>,
    text: &BufferText,
    head: ClusterStart,
    down: bool,
    count: usize,
    target_line_display_col: BufferLineCol,
) -> ClusterStart {
    let line = text.char_to_line(head.offset());
    let target_line = if down {
        // On the last content line, line + count would be the phantom
        // trailing line (the structural \n); clamp, there is nothing past it.
        line.advance(count).min(text.last_content_line())
    } else {
        line.retreat_saturating(count)
    };
    if target_line == line {
        return head; // already at the document's first/last content line
    }
    dlm.char_at_buffer_line_col(
        target_line,
        target_line_display_col,
        DisplayColTarget::NearestContent,
    )
}

/// How `apply_visual_vertical`'s `count` should be interpreted.
pub(super) enum VerticalMove {
    /// `count` buffer lines: `j`/`k` with an explicit numeric prefix
    /// (matches relative-line-number gutters even while wrapping).
    BufferLine,
    /// `count` real content display lines; virtual display lines are free.
    /// Plain `j`/`k` with no explicit count.
    ContentDisplayLine,
}

/// Shared core for the `j`/`k`-family visual-line movement `EditorCmd`s.
/// Screen-relative scroll (page/half-page, mouse wheel) carries its cursor
/// through `commands::scroll_view`'s own pass instead: a view command, not
/// a motion, so it does not share this function (see `move_vertical`'s doc).
pub(super) fn apply_visual_vertical(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    count: usize,
    down: bool,
    mode: MotionMode,
    unit: VerticalMove,
) {
    // Every unit now resolves its column through `DisplayLineMap`
    // (`ContentDisplayLine` via `move_vertical`'s display-line walk,
    // `BufferLine` (`9j`/`9k`) via `move_buffer_line`'s direct line jump), so
    // both latch a column from the same authority. `StickyDisplayCol`'s two
    // variants still distinguish what the column is measured *from*: a
    // wrapped `DisplayLine` latch is display-line-relative and a
    // `BufferLine` latch is buffer-line-relative, and the two coincide only
    // when nothing wraps (see `StickyDisplayCol`'s own doc).
    let is_buffer_line = matches!(unit, VerticalMove::BufferLine);

    let buf_id = view.panes[pid].buffer_id;
    // Whether this call's own latches are `BufferLine`-family: always true
    // for `VerticalMove::BufferLine`, and also true with wrapping off
    // (display-line-relative and buffer-line-relative coincide there, so
    // standardizing on `BufferLine` lets a counted `9j` and a plain `j`
    // share one latch on an unwrapped buffer). Resolved once per call, and
    // before the display-line map takes the pane mutably.
    let wrapping =
        effective_wrap_mode(state.buffers.get(buf_id), &state.settings, &view.panes[pid])
            .is_wrapping();
    let treat_as_line = is_buffer_line || !wrapping;
    let key = state.format_key(&view.panes[pid]);
    let target_display_cols = &mut state.visual_move_target_display_cols;
    target_display_cols.clear();
    let (mut dlm, _) = pane_display_lines(state.buffers.get(buf_id), &mut view.panes[pid], key);

    // Not `apply_focused_motion`: the closure also captures the display-line
    // map and the sticky-column buffer, disjoint fields of `state` that must
    // be borrowed separately from `state.panes`.
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, pid, buf_id, |st| {
        // Pass 1: resolve each selection's sticky display column. A
        // latch matching this call's own family (`BufferLine` when
        // `treat_as_line`, `DisplayLine` at the current wrap width
        // otherwise) is reused as-is; any other latch (the other
        // family, or a `DisplayLine` latch from a stale wrap geometry)
        // is re-derived instead, the same as no latch at all.
        let current_wrap_width = dlm.resolved_wrap_width();
        target_display_cols.extend(st.view().iter().map(|sel| {
            match sel.selection().sticky_display_col() {
                Some(StickyDisplayCol::BufferLine { display_col }) if treat_as_line => {
                    StickyDisplayCol::BufferLine { display_col }
                }
                Some(StickyDisplayCol::DisplayLine {
                    display_col,
                    wrap_width,
                }) if !treat_as_line && wrap_width == current_wrap_width => {
                    StickyDisplayCol::DisplayLine {
                        display_col,
                        wrap_width,
                    }
                }
                _ if treat_as_line => StickyDisplayCol::BufferLine {
                    display_col: dlm.buffer_line_col(sel.head()),
                },
                _ => StickyDisplayCol::DisplayLine {
                    display_col: dlm.locate(sel.head()).1,
                    wrap_width: current_wrap_width,
                },
            }
        }));

        // Pass 2: rebuild each selection, resolving its new head from the
        // sticky column pass 1 just latched and preserving that column so
        // consecutive presses in the same family reuse it.
        let mut targets = target_display_cols.iter();
        st.map(|sel| {
            let text = sel.text();
            let &target = targets.next().expect("one column per selection");
            let head = match target {
                StickyDisplayCol::BufferLine { display_col } if is_buffer_line => {
                    move_buffer_line(&mut dlm, text, sel.head(), down, count, display_col)
                }
                // No-wrap (`treat_as_line` without `is_buffer_line`):
                // display-line-relative and buffer-line-relative columns
                // coincide, and pass 1 resolved this latch via
                // `buffer_line_col` in that case:
                // `as_display_line_unwrapped` is the sound
                // reinterpretation `move_vertical` needs.
                StickyDisplayCol::BufferLine { display_col } => move_vertical(
                    &mut dlm,
                    sel.head(),
                    down,
                    count,
                    display_col.as_display_line_unwrapped(),
                ),
                StickyDisplayCol::DisplayLine { display_col, .. } => {
                    move_vertical(&mut dlm, sel.head(), down, count, display_col)
                }
            };
            let anchor = if mode == MotionMode::Extend {
                sel.anchor()
            } else {
                head
            };
            Selection::new(anchor, head).with_sticky(target)
        })
    });
}

// ---------------------------------------------------------------------------
// Vertical selection copy
// ---------------------------------------------------------------------------

/// Where the copy of a selection end `end` lands on line `line`: the line's
/// `\n` when `end` is on its own line's `\n`, so a copied whole line stays a
/// whole line, otherwise the original's display column.
fn copied_end(
    dlm: &mut DisplayLineMap<'_>,
    text: &BufferText,
    end: ClusterStart,
    display_col: BufferLineCol,
    line: usize,
) -> ClusterStart {
    let line = hume_rope::line::ContentLine::new(line);
    if end == line_break(text, text.char_to_line(end.offset())) {
        return line_break(text, line);
    }
    dlm.char_at_buffer_line_col(line, display_col, DisplayColTarget::NearestContent)
}

/// Duplicate each selection onto each of the `count` lines below it (`down:
/// true`) or above it, landing each copy on the original's display column.
/// Needs a `DisplayLineMap`, so it lives here rather than in `hume-ops`.
///
/// `DisplayColTarget::NearestContent` clamps to the last real character of a
/// short line and lands on `\n` only when the line is empty.
///
/// Copies step by the selection's own line span, since a one-line step would
/// make a multi-line copy overlap its source and merge with it. `count` is
/// clamped to the whole spans that fit inside the buffer. Each copy's column
/// is derived from the original, not the previous copy, so one short line
/// clamps only its own copy. The primary moves to the furthest copy of the
/// original primary, or stays put if no copy was added.
fn copy_selection_vertically(
    dlm: &mut DisplayLineMap<'_>,
    state: EditState,
    down: bool,
    count: usize,
) -> EditState {
    let direction: isize = if down { 1 } else { -1 };
    let view = state.view();
    let text = view.text();
    // Collect originals into `all_sels`. Copies are appended below.
    let mut all_sels: Vec<Selection> = view.iter().map(|s| s.selection()).collect();
    // Index in `all_sels` for the furthest copy of the old primary, if one was added.
    let mut primary_copy_idx: Option<usize> = None;

    // Line indices below are bare `isize`, deliberately: `direction` (+1/-1)
    // has to multiply uniformly into `anchor_line`/`head_line`/`outer_line`
    // regardless of copy direction, and `ContentLine::down`/`up` split that
    // one signed step into a per-direction branch at every use instead.
    // `available`'s own division is exactly the same shape one line down.
    // Every value here stays `<= last_content_line()` by construction (each
    // is a real selection's line, or that line shifted by a `steps` already
    // bounded by `available`), so the later `as usize` cast back into
    // `ContentLine::new` below is never out of range.
    for sel in view.iter() {
        let anchor_line = text.char_to_line(sel.anchor().offset()).index() as isize;
        let head_line = sel.head_line().index() as isize;

        // The outermost line in the copy direction determines the offset target.
        let outer_line = if down {
            anchor_line.max(head_line) // bottommost for "down"
        } else {
            anchor_line.min(head_line) // topmost for "up"
        };
        let span = (anchor_line - head_line).unsigned_abs() as isize + 1;

        // Both endpoints' display columns are loop-invariant (the original
        // selection never changes across copies), so compute them once
        // instead of re-deriving on every iteration.
        let anchor_display_col = dlm.buffer_line_col(sel.anchor());
        let head_display_col = dlm.buffer_line_col(sel.head());

        // How many whole `span`-line steps fit between the selection and the
        // buffer's edge in `direction`, floor-divided: the last step that
        // still lands fully on real content. Kept in `usize` throughout: a
        // `count` of `usize::MAX` must clamp here without ever appearing in
        // an `isize` computation, which `available.min(count)` guarantees.
        let available = if down {
            (text.last_content_line().index() as isize - outer_line).max(0) / span
        } else {
            outer_line / span
        } as usize;
        let steps = available.min(count);

        for step in 1..=steps {
            let delta = step as isize * span * direction;
            let new_anchor = copied_end(
                dlm,
                text,
                sel.anchor(),
                anchor_display_col,
                (anchor_line + delta) as usize,
            );
            let new_head = copied_end(
                dlm,
                text,
                sel.head(),
                head_display_col,
                (head_line + delta) as usize,
            );

            if sel.is_primary() {
                primary_copy_idx = Some(all_sels.len());
            }
            all_sels.push(Selection::new(new_anchor, new_head));
        }
    }

    let desired_primary = primary_copy_idx.unwrap_or(view.primary().index());
    state.with_selections(all_sels, desired_primary)
}

/// Shared body of [`cmd_copy_selection_on_next_line`]/[`cmd_copy_selection_on_prev_line`].
///
/// Builds the `DisplayLineMap` [`copy_selection_vertically`] needs before entering
/// `apply_doc_motion`. One `DisplayLineMap` line-format per selection per target
/// line. For a lone cursor this is the same per-line cost `9j`/`9k` already
/// pay for the same reason (a rope-only column can't see tabs or the
/// decoration layer); with `count` copies of several selections the cost
/// multiplies, since `DisplayLineMap` caches only the one line it last formatted.
fn copy_selection_on_line(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    down: bool,
) {
    let buf_id = t.bid(view);
    let key = state.format_key(&view.panes[t.pid()]);
    let (mut dlm, _) = pane_display_lines(state.buffers.get(buf_id), &mut view.panes[t.pid()], key);

    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        t.pid(),
        buf_id,
        |st| copy_selection_vertically(&mut dlm, st, down, count),
    );
}

// ---------------------------------------------------------------------------
// Public commands
// ---------------------------------------------------------------------------

/// Shared body of [`cmd_visual_move_down`]/[`cmd_visual_move_up`]: the two
/// differ only in `down`, so they delegate here rather than each carrying
/// their own copy of the count-unit decision.
fn visual_move_vertical(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    down: bool,
    mode: MotionMode,
) {
    // A count typed by the user (e.g. `9j`) means "9 buffer lines" (matching
    // relative-line-number gutters) even when soft-wrap is on.
    let unit = if state.explicit_count {
        VerticalMove::BufferLine
    } else {
        VerticalMove::ContentDisplayLine
    };
    apply_visual_vertical(state, view, t.pid(), count, down, mode, unit);
}

pub(super) fn cmd_visual_move_down(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    visual_move_vertical(state, view, t, count, true, mode);
    Ok(())
}

pub(super) fn cmd_visual_move_up(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    visual_move_vertical(state, view, t, count, false, mode);
    Ok(())
}

/// Duplicate each selection on the line below.
pub(super) fn cmd_copy_selection_on_next_line(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    copy_selection_on_line(state, view, t, count, true);
    Ok(())
}

/// Duplicate each selection on the line above.
pub(super) fn cmd_copy_selection_on_prev_line(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    copy_selection_on_line(state, view, t, count, false);
    Ok(())
}

/// Wrap-aware variant of `select-word-nearest-on-line`.
///
/// When wrap is active, scopes the nearest-word search to the selection
/// anchor's current display line rather than the full buffer line, matching
/// `cmd_select_word_nearest_on_line`'s own use of `sel.anchor()`. This
/// prevents the search from finding words that live on an adjacent display
/// line when the anchor lands on leading whitespace near a wrap boundary:
/// the failure mode that causes `j`/`k` bindings to oscillate in place.
///
/// Falls back to `cmd_select_word_nearest_on_line` (buffer-line bounds) when
/// wrap is off, producing identical behaviour.
pub(super) fn cmd_visual_select_word_nearest_on_line(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let doc = state.buffers.get(buf_id);
    let around = doc.overrides.word_selects_whitespace(&state.settings);
    // Owned, not borrowed: the no-wrap branch below calls
    // `apply_pane_motion(state, ...)`, which takes `&mut EditorState` as
    // one opaque argument. A live borrow into `state.buffers`/`state.settings`
    // (what a borrowed `chars` would be) can't survive across that call.
    let word_chars = word_chars_owned(doc, &state.settings);
    let chars = WordChars::new(&word_chars);
    let ctx = WordCtx {
        mode,
        around,
        chars,
    };

    if !effective_wrap_mode(doc, &state.settings, &view.panes[t.pid()]).is_wrapping() {
        apply_pane_motion(state, view, t, |st| {
            cmd_select_word_nearest_on_line(st, 0, ctx)
        });
        return Ok(());
    }

    let key = state.format_key(&view.panes[t.pid()]);
    let (mut dlm, _) = pane_display_lines(state.buffers.get(buf_id), &mut view.panes[t.pid()], key);

    // Not `apply_pane_motion`: the closure also captures the display-line map.
    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        t.pid(),
        buf_id,
        |st| {
            st.map(|sel| {
                let text = sel.text();
                let pos = dlm.locate_display_line(sel.anchor());
                let bounds = dlm
                    .content_display_line_clusters(pos)
                    .unwrap_or_else(|| line_range(text, text.char_to_line(sel.anchor().offset())));
                let found = nearest_word_on_line(
                    text,
                    sel.anchor(),
                    bounds.start().into(),
                    bounds.end(),
                    around,
                    chars,
                );
                apply_nearest_word_result(sel, found, mode)
            })
        },
    );

    Ok(())
}
