//! The `Sift` layer: the `s`-prompt (sift-within) minibuffer mode.

use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
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
    /// This session's pane and its pre-entry selection snapshot: same
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
    /// Captures the snapshot here rather than at construction; see
    /// `SearchLayer::setup`'s doc for why the ordering matters.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        self.snap.capture(state, view);
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        self.snap
            .take_restore(&mut state.panes.state, &state.buffers);
        // Sift has no history ring of its own (`begin_session_all`
        // only touches the command/search rings, so this is a no-op
        // for Sift), but every other minibuf-backed mode's teardown
        // calls it unconditionally, and Sift stays uniform with
        // them rather than being special-cased as the one mode that
        // skips it.
        state.history.begin_session_all();
    }
    fn snapshot_mut(&mut self) -> Option<&mut PaneSnapshot> {
        Some(&mut self.snap)
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
            // Do NOT write to the search register or clear search state:
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
        // Up/Down are reserved for minibuffer history: no-op in sift-within.
        MiniBufferEvent::CursorMoved
        | MiniBufferEvent::Ignored
        | MiniBufferEvent::CompleteRequested { .. }
        | MiniBufferEvent::HistoryPrev
        | MiniBufferEvent::HistoryNext => {}
    }
}

/// Replace selections with the matches within the original selections
/// ([`sift_within`]). Called on every keystroke in Sift mode.
fn update_live_sift(ed: &mut Editor, r: LayerRef) {
    let pattern = match ed.state.input.minibuf() {
        Some(mb) if !mb.input.is_empty() => mb.input.clone(),
        _ => return,
    };

    let Some(sift) = ed.state.input.at::<SiftLayer>(r) else {
        return;
    };
    let pane = sift.snap.pane();
    let Some((bid, sels)) = sift.snap.selections().map(|(bid, s)| (bid, s.clone())) else {
        return;
    };

    match sift_within(ed.state.buffers.get(bid).text(), sels, &pattern) {
        Ok(new_state) => {
            let text = new_state.text().clone();
            ed.state.panes.state[pane][bid].set_selections(new_state.into_selections(), &text);
        }
        // Invalid regex in progress, or nothing matched: restore originals.
        Err(SiftFailure::Invalid | SiftFailure::NoMatch) => restore_sift_snapshot(ed, r),
    }
}

/// Why [`sift_within`] left the selections alone.
pub(in crate::editor) enum SiftFailure {
    /// The input doesn't compile to a regex.
    Invalid,
    /// No selection contains a match.
    NoMatch,
}

/// Narrow each of `sels` to the matches of `raw` inside it, parsed with the
/// search prompt's flag grammar: `v` (verbatim) literalizes the pattern the
/// same way it does in search, `m` (multi) parses but is inert, since sift
/// already operates on every selection.
pub(in crate::editor) fn sift_within(
    text: &BufferText,
    sels: SelectionSet,
    raw: &str,
) -> Result<EditState, SiftFailure> {
    let (_, regex) = compile_search_input(raw).ok_or(SiftFailure::Invalid)?;
    sift_matches_within(&EditState::bind(text, sels), &regex).ok_or(SiftFailure::NoMatch)
}

// ── Snapshot restore helpers ────────────────────────────────────────────────

/// Restore selections from the sift-mode snapshot without consuming it.
/// Always targets the session's own originating pane, not whatever's
/// currently focused (see [`PaneSnapshot`]'s own doc).
fn restore_sift_snapshot(ed: &mut Editor, r: LayerRef) {
    let Some(sift) = ed.state.input.at::<SiftLayer>(r) else {
        return;
    };
    sift.snap
        .restore(&mut ed.state.panes.state, &ed.state.buffers);
}
