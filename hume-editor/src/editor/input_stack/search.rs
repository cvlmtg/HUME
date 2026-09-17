//! The `Search` layer — the `/`/`?`-prompt minibuffer mode.

use std::sync::Arc;

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::search::{SearchDirection, compile_search_regex, find_next_match};

use super::super::commands::search_sel;
use super::super::jump_list::JumpEntry;
use super::super::minibuf::history::{HistoryDir, HistoryStore};
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::search::SearchPattern;
use super::super::{Editor, EditorState, commands, search};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef};

pub(in crate::editor) struct SearchLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
}

impl Layer for SearchLayer {
    fn handler(&self) -> LayerHandler {
        search_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Search)
    }
    /// Empty — see `InsertLayer::setup`'s doc.
    fn setup(&mut self, _state: &mut EditorState, _view: &EngineView) {}
    fn tear_down(&mut self, state: &mut EditorState, view: &EngineView) {
        let pid = state.focus.id();
        if let Some(sels) = state.panes.transient[pid].pre_search_sels.take() {
            commands::set_current_selections(state, view, sels);
            let bid = commands::focused_buffer_id(state, view);
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
            let pid = ed.state.focus.id();
            if let Some(sels) = ed.state.panes.transient[pid].pre_search_sels.take() {
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
            restore_search_snapshot(ed);
            let bid = ed.focused_buffer_id();
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
            update_live_search(ed);
        }
        MiniBufferEvent::HistoryPrev => {
            let Some(prompt) = ed.state.input.minibuf().map(|m| m.prompt.clone()) else {
                return;
            };
            let Some(kind) = HistoryStore::kind_for_prompt(&prompt) else {
                return;
            };
            minibuf::recall_history(ed, kind, HistoryDir::Prev);
            update_live_search(ed);
        }
        MiniBufferEvent::HistoryNext => {
            let Some(prompt) = ed.state.input.minibuf().map(|m| m.prompt.clone()) else {
                return;
            };
            let Some(kind) = HistoryStore::kind_for_prompt(&prompt) else {
                return;
            };
            minibuf::recall_history(ed, kind, HistoryDir::Next);
            update_live_search(ed);
        }
        MiniBufferEvent::CursorMoved
        | MiniBufferEvent::Ignored
        | MiniBufferEvent::CompleteRequested { .. } => {}
    }
}

/// Recompile the regex from the current mini-buffer input and jump to the
/// first match from the pre-search position.
///
/// Called on every keystroke while in Search mode.
fn update_live_search(ed: &mut Editor) {
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
    let pid = ed.state.focus.id();

    // Start from the original pre-search position (not the current position),
    // so each additional character refines from the same anchor point.
    let from_char = {
        let pt = &ed.state.panes.transient[pid];
        match &pt.pre_search_sels {
            Some(sels) => {
                let text = ed.doc().text();
                let primary = sels.primary();
                match direction {
                    SearchDirection::Forward => primary.start(),
                    SearchDirection::Backward => primary.end_inclusive(text),
                }
            }
            None => hume_rope::offset::CharOffset::new(0),
        }
    };

    match find_next_match(ed.doc().text(), &regex, from_char, direction) {
        Some((span, _wrapped)) => {
            let anchor = if ed.state.panes.transient[pid].search_extend {
                // Extend from the original anchor.
                Some(
                    ed.state.panes.transient[pid]
                        .pre_search_sels
                        .as_ref()
                        .map(|s| s.primary().anchor())
                        .unwrap_or(span.start),
                )
            } else {
                None
            };
            ed.set_primary_selection(search_sel(span, anchor, direction));
        }
        None => {
            // No match — restore position to pre-search.
            restore_search_snapshot(ed);
        }
    }

    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
        regex: Arc::new(regex),
        pattern_str: pattern,
    });
}

// ── Snapshot restore helpers ────────────────────────────────────────────────

/// Restore selections from the search-mode snapshot without consuming it.
fn restore_search_snapshot(ed: &mut Editor) {
    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    // pane_transient and pane_state are disjoint fields — no &mut ed needed.
    if let Some(sels) = ed.state.panes.transient[pid].pre_search_sels.as_ref() {
        ed.state.panes.state[pid][bid].set_selections(sels.clone());
    }
}
