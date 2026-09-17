//! The `Search` layer — the `/`/`?`-prompt minibuffer mode.

use std::sync::Arc;

use hume_engine::pipeline::{EngineView, PaneId};
use hume_engine::types::EditorMode;
use hume_ops::search::{SearchDirection, compile_search_regex, find_next_match};

use super::super::commands::search_sel;
use super::super::jump_list::JumpEntry;
use super::super::minibuf::history::{HistoryDir, HistoryStore};
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::search::SearchPattern;
use super::super::{Editor, EditorState, commands, search};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};
use hume_editing::selection::SelectionSet;

pub(in crate::editor) struct SearchLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    /// The pane this session opened in — restore/clear always targets this,
    /// never `state.focus.id()`, which may have moved on since: a mouse
    /// click always falls through under this layer (`minibuf_input`'s
    /// `Mouse` arm), so focus (and whatever it opens next) can move to a
    /// different pane while Search stays open.
    pub(in crate::editor) pane: PaneId,
    /// Snapshot of `pane`'s selections before this session opened, for
    /// cancel-restore. `None` once the `Confirm` arm has taken it — see its
    /// own comment for why teardown must then no-op.
    pub(in crate::editor) pre_sels: Option<SelectionSet>,
    /// Whether Extend mode was active when this session opened. Captured so
    /// live-search can extend from the pre-search anchor even though `mode`
    /// is `Search` during the live preview.
    pub(in crate::editor) extend: bool,
}

impl Layer for SearchLayer {
    fn handler(&self) -> LayerHandler {
        search_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Search)
    }
    /// Captures `pre_sels` here rather than at construction: `setup` runs
    /// after the outgoing layer's own `tear_down` (see the `Layer::setup`
    /// doc), so a re-entrant `/`-search (`push_mode_layer` replaces rather
    /// than no-ops on same-kind re-entry for every mode layer but `Insert`)
    /// snapshots the state the outgoing session's `tear_down` just restored
    /// — the true pre-search selections — instead of the mid-search preview
    /// a construction-time capture would have caught.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        self.pre_sels = Some(commands::current_selections(state, view).clone());
    }
    fn tear_down(&mut self, state: &mut EditorState, view: &EngineView, _why: Removal) {
        if let Some(sels) = self.pre_sels.take() {
            let bid = view.panes[self.pane].buffer_id;
            state.panes.state[self.pane][bid].set_selections(sels);
            search::ops::clear_buffer_search(&mut state.buffers, &mut state.panes.state, bid);
        }
        state.history.begin_session_all();
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

pub(in crate::editor) fn search_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::minibuf_input(ed, r, ev) {
        handle_search_event(ed, r, event);
    }
}

fn handle_search_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => {
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Confirm(pattern) => {
            // Record into the correct search ring before truncating.
            let kind = ed
                .state
                .input
                .minibuf()
                .and_then(|m| HistoryStore::kind_for_prompt(&m.prompt));
            if let Some(k) = kind {
                ed.state.history.get_mut(k).push(pattern.clone());
            }
            // Persist pattern in 's' register for future n/N.
            ed.state.registers.set_search_register(pattern);
            // Record the pre-search position in the jump list before
            // discarding it, unless the match confirmed is the position
            // search started from (record_jump_if_moved). The `Confirm`
            // arm does its own accept work — taking the stash — before
            // truncating; teardown's `Search` arm restores it on every
            // *other* removal, so it's already gone here and would be a
            // no-op if left to teardown.
            let pre_sels = ed
                .state
                .input
                .at_mut::<SearchLayer>(r)
                .and_then(|s| s.pre_sels.take());
            if let Some(sels) = pre_sels {
                let bid = ed.focused_buffer_id();
                let entry = JumpEntry::new(sels, ed.doc().text(), bid);
                commands::record_jump_if_moved(&mut ed.state, &ed.view, entry);
            }
            // search_pattern stays alive on the buffer for immediate n/N
            // without recompile — the stash is already taken above, so
            // teardown's `Search` arm (gated on the same stash) won't
            // clear it.
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::EmptiedByBackspace => {
            // First Backspace cleared the last character — restore position but
            // stay in Search mode. A second Backspace (BackspaceOnEmpty) dismisses.
            restore_search_snapshot(ed, r);
            let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
                return;
            };
            let bid = ed.view.panes[search.pane].buffer_id;
            search::ops::clear_buffer_search(&mut ed.state.buffers, &mut ed.state.panes.state, bid);
        }
        MiniBufferEvent::BackspaceOnEmpty => {
            // Input already empty — user pressed Backspace a second time to
            // dismiss. Teardown restores the snapshot, clears the search, and
            // begins a fresh history session — the same as this arm's own
            // body used to.
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Edited => {
            if let Some(k) = ed
                .state
                .input
                .minibuf()
                .and_then(|m| HistoryStore::kind_for_prompt(&m.prompt))
            {
                ed.state.history.get_mut(k).demote_to_scratch();
            }
            update_live_search(ed, r);
        }
        MiniBufferEvent::HistoryPrev => recall_search_history(ed, r, HistoryDir::Prev),
        MiniBufferEvent::HistoryNext => recall_search_history(ed, r, HistoryDir::Next),
        MiniBufferEvent::CursorMoved
        | MiniBufferEvent::Ignored
        | MiniBufferEvent::CompleteRequested { .. } => {}
    }
}

/// Recalls the previous/next entry from whichever ring `dir` names — shared
/// by the `HistoryPrev`/`HistoryNext` arms above, which differ only in
/// `dir` — then refreshes the live preview against the recalled pattern.
fn recall_search_history(ed: &mut Editor, r: LayerRef, dir: HistoryDir) {
    let Some(prompt) = ed.state.input.minibuf().map(|m| m.prompt.clone()) else {
        return;
    };
    let Some(kind) = HistoryStore::kind_for_prompt(&prompt) else {
        return;
    };
    minibuf::recall_history(ed, kind, dir);
    update_live_search(ed, r);
}

/// Recompile the regex from the current mini-buffer input and jump to the
/// first match from the pre-search position.
///
/// Called on every keystroke while in Search mode. Targets the *focused*
/// pane/buffer, same as before this session's stash moved onto
/// `SearchLayer` — a live preview while typing has always followed focus,
/// unlike cancel-restore/clear below, which must target the session's own
/// originating pane instead (see `SearchLayer::pane`'s doc).
fn update_live_search(ed: &mut Editor, r: LayerRef) {
    let pattern = match ed.state.input.minibuf() {
        Some(mb) if !mb.input.is_empty() => mb.input.clone(),
        _ => return,
    };

    let Some(regex) = compile_search_regex(&pattern) else {
        // Invalid regex in progress — clear pattern so highlights disappear.
        let bid = ed.focused_buffer_id();
        search::ops::clear_buffer_search(&mut ed.state.buffers, &mut ed.state.panes.state, bid);
        return;
    };

    let direction = ed.state.search.direction;
    let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
        return;
    };
    let extend = search.extend;
    // Read the two `CharOffset`s this function actually needs straight out
    // of the borrow — `ed.doc()` is a second, independent shared borrow of
    // `ed` (same shape as `update_live_sift`'s `ed.doc()` call inside its
    // own `sift.pre_sels.as_ref()` closure), so it can run while `search`
    // is still alive. Cloning the whole `SelectionSet` here, just to read
    // one or two of its offsets, would cost a `Vec` allocation on every
    // keystroke typed into the search prompt.
    let anchor = search.pre_sels.as_ref().map(|s| s.primary().anchor());
    let from_char = match search.pre_sels.as_ref() {
        Some(sels) => {
            let text = ed.doc().text();
            let primary = sels.primary();
            match direction {
                SearchDirection::Forward => primary.start(),
                SearchDirection::Backward => primary.end_inclusive(text),
            }
        }
        None => hume_rope::offset::CharOffset::new(0),
    };

    match find_next_match(ed.doc().text(), &regex, from_char, direction) {
        Some((span, _wrapped)) => {
            // Extend from the original anchor.
            let anchor = extend.then(|| anchor.unwrap_or(span.start));
            ed.set_primary_selection(search_sel(span, anchor, direction));
        }
        None => {
            // No match — restore position to pre-search.
            restore_search_snapshot(ed, r);
        }
    }

    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
        regex: Arc::new(regex),
        pattern_str: pattern,
    });
}

// ── Snapshot restore helpers ────────────────────────────────────────────────

/// Restore selections from the search-mode snapshot without consuming it —
/// always targets the session's own originating pane (`SearchLayer::pane`),
/// not whatever's currently focused.
fn restore_search_snapshot(ed: &mut Editor, r: LayerRef) {
    let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
        return;
    };
    let Some(sels) = search.pre_sels.clone() else {
        return;
    };
    let pane = search.pane;
    let bid = ed.view.panes[pane].buffer_id;
    ed.state.panes.state[pane][bid].set_selections(sels);
}
