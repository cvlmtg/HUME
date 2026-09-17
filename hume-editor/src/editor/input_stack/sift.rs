//! The `Sift` layer — the `s`-prompt (sift-within) minibuffer mode.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::search::compile_search_regex;
use hume_ops::selection_cmd::sift_matches_within;

use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::{Editor, EditorState, commands};
use super::stack::{InputEvent, Layer, LayerRef};

pub(in crate::editor) struct SiftLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
}

impl Layer for SiftLayer {
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Sift)
    }
    fn tear_down(&mut self, state: &mut EditorState, view: &EngineView) {
        let pid = state.focus.id();
        if let Some(sels) = state.panes.transient[pid].pre_sift_sels.take() {
            commands::set_current_selections(state, view, sels);
        }
        // Sift has no history ring of its own — `begin_session_all`
        // only touches the command/search rings, so this is a no-op
        // for Sift — but every other minibuf-backed mode's teardown
        // calls it unconditionally, and Sift stays uniform with
        // them rather than being special-cased as the one mode that
        // skips it.
        state.history.begin_session_all();
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

pub(in crate::editor) fn sift_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::minibuf_input(ed, r, ev) {
        handle_sift_event(ed, r, event);
    }
}

fn handle_sift_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => {
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Confirm(_) => {
            // Keep the selections that live preview already set. This
            // `Confirm` arm clears its own stash before truncating, so
            // teardown's `Sift` arm (which would otherwise restore it)
            // finds nothing to do.
            let pid = ed.state.focus.id();
            ed.state.panes.transient[pid].pre_sift_sels = None;
            // Do NOT write to the search register or clear search state —
            // sift-within is a selection op, not a search. The previous
            // search pattern and its highlights should be preserved so that
            // n/N continues to navigate the original search.
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::EmptiedByBackspace | MiniBufferEvent::BackspaceOnEmpty => {
            // Restore original selections when pattern is fully erased.
            restore_sift_snapshot(ed);
        }
        MiniBufferEvent::Edited => update_live_sift(ed),
        // Up/Down are reserved for minibuffer history — no-op in sift-within.
        MiniBufferEvent::CursorMoved
        | MiniBufferEvent::Ignored
        | MiniBufferEvent::CompleteRequested { .. }
        | MiniBufferEvent::HistoryPrev
        | MiniBufferEvent::HistoryNext => {}
    }
}

/// Recompile the regex and replace selections with matches within the
/// original selections. Called on every keystroke in Sift mode.
fn update_live_sift(ed: &mut Editor) {
    let pattern = match ed.state.input.minibuf() {
        Some(mb) if !mb.input.is_empty() => mb.input.clone(),
        _ => return,
    };

    let Some(regex) = compile_search_regex(&pattern) else {
        // Invalid regex in progress — restore originals.
        restore_sift_snapshot(ed);
        return;
    };

    // Compute matches in a limited scope so the borrow on
    // pre_sift_sels is released before we need to restore.
    let pid = ed.state.focus.id();
    let result = ed.state.panes.transient[pid]
        .pre_sift_sels
        .as_ref()
        .and_then(|sels| sift_matches_within(ed.doc().text(), sels, &regex));

    match result {
        Some(new_sels) => ed.set_current_selections(new_sels),
        None => restore_sift_snapshot(ed),
    }
}

// ── Snapshot restore helpers ────────────────────────────────────────────────

/// Restore selections from the sift-mode snapshot without consuming it.
fn restore_sift_snapshot(ed: &mut Editor) {
    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    // pane_transient and pane_state are disjoint fields — no &mut ed needed.
    if let Some(sels) = ed.state.panes.transient[pid].pre_sift_sels.as_ref() {
        ed.state.panes.state[pid][bid].set_selections(sels.clone());
    }
}
