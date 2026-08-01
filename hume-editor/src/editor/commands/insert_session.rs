//! Insert-mode session lifecycle: entering/exiting Insert as a repeatable
//! action, with the undo group and dot-repeat bookkeeping that entails.

use hume_editing::selection::Selection;
use hume_engine::pipeline::EngineView;

use crate::editor::buffer::LastInsert;
use crate::editor::replay::InsertSession;
use crate::editor::{EditorState, Mode, Severity};
use hume_ops::edit::clear_blank_line_indent;

use super::{
    apply_focused_edit_grouped, apply_focused_motion, begin_edit_group_current,
    commit_edit_group_current, current_selections, doc, focused_buffer_id,
    focused_buffer_read_only,
};

/// `true` when the focused (pane, buffer) has an open edit group.
pub(super) fn is_group_open_current(state: &EditorState, view: &EngineView) -> bool {
    let bid = focused_buffer_id(state, view);
    state.panes.state[state.focused_pane_id][bid]
        .edit_group
        .is_some()
}

/// `true` if any current selection is a collapsed cursor sitting on a blank,
/// auto-indented line — the condition under which [`clear_blank_line_indent`]
/// would actually change the buffer. Checked before calling it so the common
/// case (exiting Insert mode away from a blank line) skips the edit entirely
/// instead of running an identity one (see
/// [`hume_ops::edit::blank_line_ws_range`]'s doc comment).
pub(super) fn has_blank_line_cursor(state: &EditorState, view: &EngineView) -> bool {
    let buf = doc(state, view).text();
    current_selections(state, view).iter_sorted().any(|sel| {
        sel.is_collapsed() && hume_ops::edit::blank_line_ws_range(buf, sel.head()).is_some()
    })
}

/// Pin each current selection's head as an insertion anchor, so `mii`
/// (`select-last-insertion`) — and, for a `c`-entered session with
/// `select-changed-text` on, `end_insert_session` itself — can later
/// reconstruct the span(s) typed since.
///
/// No-op if no edit group is open (read-only buffer, where
/// `begin_insert_session` already refused to enter Insert). Call after the
/// cursor has been positioned at the insertion point — for `o`/`O`, after the
/// structural newline has been inserted — so the anchor marks the start of
/// typed text only, never the newline or the pre-edit selection.
///
/// `apply_doc_edit_grouped` (doc_ops.rs) maps the pinned anchors through
/// every subsequent grouped edit; a cursor-motion key during the session
/// clears them (`mappings/insert.rs`); a fresh `begin_edit_group` clears them
/// too, so a later session never inherits stale pins.
pub(super) fn pin_insert_anchors(state: &mut EditorState, view: &EngineView) {
    if !is_group_open_current(state, view) {
        return;
    }
    let anchors: Vec<usize> = current_selections(state, view)
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    state.panes.state[pid][bid].pinned_anchors = Some(anchors);
}

/// Enter Insert mode as a repeatable insert action.
///
/// No-op (with a warning) if the focused buffer is read-only. Replay-signal:
/// if an edit group is already open, recording is suppressed but the mode
/// change still happens.
pub(super) fn begin_insert_session(state: &mut EditorState, view: &EngineView) {
    if focused_buffer_read_only(state, view) {
        state.report(Severity::Info, "Buffer is read-only".to_string());
        return;
    }
    // Guard is load-bearing for dot-repeat replay: `replay_dot` opens
    // an edit group before re-dispatching the command, so a group already being
    // open here means "we are replaying" → skip session creation and re-type from
    // `insert_keys` instead of recording fresh. Do NOT weaken this into a separate
    // flag without also fixing the replay signal.
    //
    // The implied assumption — that no Steel body can reach `begin_insert_session`
    // with a group already open outside of replay — holds because Steel has no
    // transaction / begin-edit-group builtin, and none should ever be added:
    // fine-grained undo grouping belongs to native commands, not scripts.
    if !is_group_open_current(state, view) {
        begin_edit_group_current(state, view);
        state.insert_session = Some(InsertSession {
            keystrokes: Vec::new(),
            step_back_on_exit: false,
        });
    }
    // Outside the guard above (unlike `insert_session`) so replay — which
    // skips session creation — still starts each replayed session with no
    // pending auto-indent to vacate, matching a fresh interactive session.
    state.autoindent_pending = false;
    state.set_mode(Mode::Insert);
}

/// Exit Insert mode and finalise the undo/repeat state.
pub(in crate::editor) fn end_insert_session(state: &mut EditorState, view: &EngineView) {
    let step_back = state
        .insert_session
        .as_ref()
        .is_some_and(|s| s.step_back_on_exit);
    // Vim autoindent parity: trim a blank auto-indented line's whitespace
    // before committing, so leaving Insert mode on one behaves like Enter
    // does in `insert_newline_indent`. Joins the still-open session group —
    // not a separate undo step. Gated on two conditions: `autoindent_pending`
    // (the line's indent was auto-inserted by *this* session and nothing has
    // been typed on it since — vim only vacates indent it created, never
    // pre-existing or hand-typed whitespace) and `has_blank_line_cursor` (the
    // common case, cursor not on a blank line, skips the edit rather than
    // running an identity one on every Insert-mode exit).
    if state.autoindent_pending && has_blank_line_cursor(state, view) {
        apply_focused_edit_grouped(state, view, clear_blank_line_indent);
    }
    commit_edit_group_current(state, view);
    if let (Some(session), Some(action)) = (
        state.insert_session.take(),
        state.last_repeatable_action.as_mut(),
    ) {
        action.insert_keys = session.keystrokes;
    }
    // Every insert entry pins one anchor per selection via
    // `pin_insert_anchors` — reconstruct each selection's typed span here as
    // `(anchor, head - 1 grapheme)`, guarded by `head > anchor` (nothing
    // typed / all backspaced away yields `None`, never a backwards or
    // zero-width range). A count mismatch (selections merged mid-session,
    // e.g. via Backspace) drops the pins entirely — `spans` stays `None`, so
    // this session contributes nothing to the `mii` stash and (for `c`)
    // falls back to a collapsed cursor.
    let (pinned, select_on_exit, kill_opened) = {
        let pid = state.focused_pane_id;
        let bid = focused_buffer_id(state, view);
        let pbs = &mut state.panes.state[pid][bid];
        (
            pbs.pinned_anchors.take(),
            std::mem::take(&mut pbs.select_on_exit),
            std::mem::take(&mut pbs.kill_opened_session),
        )
    };
    // `cmd_change` stamped `PasteAnchor` right after the deletion, but every
    // keystroke since has bumped `edit_seq` — refresh the stamp to the
    // session's final `seq` (source unchanged) so `c <text> <Esc> p` still
    // reads the ring. See `PaneBufferState::kill_opened_session`'s doc.
    if kill_opened && let Some(anchor) = state.paste_anchor.as_mut() {
        anchor.seq = state.buffers.edit_seq();
    }
    let valid_pins = pinned.filter(|a| a.len() == current_selections(state, view).len());
    let spans: Option<Vec<Option<(usize, usize)>>> = valid_pins.map(|anchors| {
        let buf = doc(state, view).text();
        current_selections(state, view)
            .iter_sorted()
            .zip(anchors.iter())
            .map(|(sel, &anchor)| {
                let head = sel.head();
                (head > anchor).then(|| {
                    (
                        anchor,
                        hume_editing::grapheme::prev_grapheme_boundary(buf, head),
                    )
                })
            })
            .collect()
    });

    // Stash whatever was actually typed for `mii`, regardless of entry
    // command — independent of `select_on_exit` below, which only decides
    // whether Esc *also* selects it immediately.
    if let Some(spans) = &spans {
        let stashed: Vec<(usize, usize)> = spans.iter().flatten().copied().collect();
        if !stashed.is_empty() {
            let bid = focused_buffer_id(state, view);
            let buf = state.buffers.get_mut(bid);
            let text_gen = buf.text_gen;
            buf.last_insert = Some(LastInsert {
                spans: stashed,
                text_gen,
            });
        }
    }

    if let Some(spans) = spans.filter(|_| select_on_exit) {
        apply_focused_motion(state, view, move |_b, sels| {
            let mut spans = spans.into_iter();
            sels.map(|sel| match spans.next().expect("length checked above") {
                Some((anchor, end)) => Selection::new(anchor, end),
                None => Selection::collapsed(sel.head()),
            })
        });
    } else if step_back {
        apply_focused_motion(state, view, |b, sels| {
            sels.map(|sel| {
                let head = sel.head();
                let line_start = b.line_to_char(b.char_to_line(head));
                let new_head = if head > line_start {
                    hume_editing::grapheme::prev_grapheme_boundary(b, head)
                } else {
                    head
                };
                Selection::collapsed(new_head)
            })
        });
    }
    state.set_mode(Mode::Normal);
}
