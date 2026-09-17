//! The `Sift` layer — the `s`-prompt (sift-within) minibuffer mode.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::search::compile_search_input;
use hume_ops::selection_cmd::sift_matches_within;

use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::{Editor, EditorState};
use super::snapshot::PaneSnapshot;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

pub(in crate::editor) struct SiftLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    /// This session's pane and its pre-entry selection snapshot — same
    /// reasoning as `SearchLayer::snap`: a mouse click always falls through
    /// under this layer, so focus can move to a different pane while Sift
    /// stays open. See [`PaneSnapshot`]'s own doc for the capture/restore
    /// rules.
    pub(in crate::editor) snap: PaneSnapshot,
}

impl Layer for SiftLayer {
    fn handler(&self) -> LayerHandler {
        sift_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Sift)
    }
    /// Captures the snapshot here rather than at construction — see
    /// `SearchLayer::setup`'s doc for why the ordering matters.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        self.snap.capture(state, view);
    }
    fn tear_down(&mut self, state: &mut EditorState, view: &EngineView, _why: Removal) {
        self.snap.take_restore(&mut state.panes.state, view);
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
            if let Some(sift) = ed.state.input.at_mut::<SiftLayer>(r) {
                sift.snap.take_selections();
            }
            // Do NOT write to the search register or clear search state —
            // sift-within is a selection op, not a search. The previous
            // search pattern and its highlights should be preserved so that
            // n/N continues to navigate the original search.
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::EmptiedByBackspace | MiniBufferEvent::BackspaceOnEmpty => {
            // Restore original selections when pattern is fully erased.
            restore_sift_snapshot(ed, r);
        }
        MiniBufferEvent::Edited => update_live_sift(ed, r),
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
///
/// Shares `parse_search_input`'s flag grammar with the search prompt — `v`
/// (verbatim) literalizes the pattern the same way it does in search. `m`
/// (multi) parses but is inert here: sift already operates on every selection.
fn update_live_sift(ed: &mut Editor, r: LayerRef) {
    let pattern = match ed.state.input.minibuf() {
        Some(mb) if !mb.input.is_empty() => mb.input.clone(),
        _ => return,
    };

    let Some((_, regex)) = compile_search_input(&pattern) else {
        // Invalid regex in progress — restore originals.
        restore_sift_snapshot(ed, r);
        return;
    };

    let Some(sift) = ed.state.input.at::<SiftLayer>(r) else {
        return;
    };
    let result = sift
        .snap
        .selections()
        .and_then(|sels| sift_matches_within(ed.doc().text(), sels, &regex));

    match result {
        Some(new_sels) => ed.set_current_selections(new_sels),
        None => restore_sift_snapshot(ed, r),
    }
}

// ── Snapshot restore helpers ────────────────────────────────────────────────

/// Restore selections from the sift-mode snapshot without consuming it —
/// always targets the session's own originating pane, not whatever's
/// currently focused (see [`PaneSnapshot`]'s own doc).
fn restore_sift_snapshot(ed: &mut Editor, r: LayerRef) {
    let Some(sift) = ed.state.input.at::<SiftLayer>(r) else {
        return;
    };
    sift.snap.restore(&mut ed.state.panes.state, &ed.view);
}
