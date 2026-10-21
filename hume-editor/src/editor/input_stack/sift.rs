//! The `Sift` layer: the `s`-prompt (sift-within) minibuffer mode.

use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_engine::types::EditorMode;
use hume_ops::search::compile_search_input;
use hume_ops::selection_cmd::sift_matches_within;

use super::super::error::CommandError;
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::replay::WaitingInput;
use super::super::{Editor, EditorState, commands};
use super::paste::PastedText;
use super::snapshot::PaneSnapshot;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PasteHandler, Removal};

pub(in crate::editor) struct SiftLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    /// This session's pane and its pre-entry selection snapshot: same
    /// reasoning as `SearchLayer::snap`: a mouse click always falls through
    /// under this layer, so focus can move to a different pane while Sift
    /// stays open. See [`PaneSnapshot`]'s own doc for the capture/restore
    /// rules.
    pub(in crate::editor) snap: PaneSnapshot,
    /// The `sift-within` this prompt reads the pattern for, run with it on
    /// Enter.
    pub(in crate::editor) then: commands::PendingDispatch,
    /// The live preview's last sift, `None` while the input sifts nothing.
    pub(in crate::editor) preview: Option<SiftPreview>,
    /// The check this prompt passed as it opened.
    input: WaitingInput,
}

impl SiftLayer {
    pub(in crate::editor) fn new(
        input: WaitingInput,
        minibuf: MiniBuffer,
        snap: PaneSnapshot,
        then: commands::PendingDispatch,
    ) -> Self {
        Self {
            minibuf,
            snap,
            then,
            preview: None,
            input,
        }
    }
}

/// What the live preview sifted: `from`, the snapshot's selections, by
/// `pattern`, giving `sifted`.
pub(in crate::editor) struct SiftPreview {
    pattern: String,
    from: SelectionSet,
    sifted: SelectionSet,
}

/// A confirmed preview's result, left on `EditorState::sift_reuse` for the
/// `sift-within` that Enter runs to take instead of sifting again.
pub(in crate::editor) struct SiftReuse {
    pane: PaneId,
    bid: BufferId,
    preview: SiftPreview,
}

impl SiftReuse {
    /// The preview's result when it sifted `from` on `(pane, bid)` by
    /// `pattern`. `from` carries the version of the text it was read
    /// from, so equal selections also prove an unchanged text.
    pub(in crate::editor) fn result_for(
        self,
        pane: PaneId,
        bid: BufferId,
        from: &SelectionSet,
        pattern: &str,
    ) -> Option<SelectionSet> {
        let SiftPreview {
            pattern: previewed,
            from: previewed_from,
            sifted,
        } = self.preview;
        (self.pane == pane && self.bid == bid && previewed_from == *from && previewed == pattern)
            .then_some(sifted)
    }
}

impl Layer for SiftLayer {
    fn handler(&self) -> LayerHandler {
        sift_input
    }
    fn paste_handler(&self) -> PasteHandler {
        sift_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Sift)
    }
    /// Captures the snapshot here rather than at construction; see
    /// `SearchLayer::setup`'s doc for why the ordering matters.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        self.snap.capture(state, view);
    }
    fn tear_down(mut self: Box<Self>, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        self.end(state, None);
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

impl SiftLayer {
    /// Restores the pre-sift selections. When the live preview sifted for
    /// `confirmed` itself, its result goes to `EditorState::sift_reuse`.
    fn end(&mut self, state: &mut EditorState, confirmed: Option<&str>) {
        let bid = self
            .snap
            .take_restore(&mut state.panes.entries, &state.buffers);
        if let (Some(raw), Some(bid), Some(preview)) = (confirmed, bid, self.preview.take())
            && preview.pattern == raw
        {
            state.sift_reuse = Some(SiftReuse {
                pane: self.snap.pane(),
                bid,
                preview,
            });
        }
        // Sift has no history ring of its own, but every other
        // minibuf-backed mode's teardown resets every ring
        // (`begin_session_all`) as it closes, and Sift does the same.
        state.history.begin_session_all();
    }
}

pub(in crate::editor) fn sift_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::minibuf_input(ed, r, ev) {
        handle_sift_event(ed, r, event);
    }
}

fn sift_paste(
    state: &mut EditorState,
    _view: &EngineView,
    r: LayerRef,
    text: PastedText,
) -> Result<(), CommandError> {
    if minibuf::paste_into_minibuf(state, &text) {
        update_live_sift(state, r);
    }
    Ok(())
}

fn handle_sift_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => {
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Confirm(pattern) => {
            // The command sifts the selections the prompt opened over, as a
            // `call!` would.
            let mut sift = ed.state.take_layer::<SiftLayer>(&ed.view, r);
            sift.end(&mut ed.state, Some(&pattern));
            commands::finish_text_operand(ed, sift.then, sift.input.take_answer(), Some(pattern));
        }
        MiniBufferEvent::EmptiedByBackspace | MiniBufferEvent::BackspaceOnEmpty => {
            // Restore original selections when pattern is fully erased.
            restore_sift_snapshot(&mut ed.state, r);
        }
        MiniBufferEvent::Edited => update_live_sift(&mut ed.state, r),
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
fn update_live_sift(state: &mut EditorState, r: LayerRef) {
    let pattern = match state.input.minibuf() {
        Some(mb) if !mb.input.is_empty() => mb.input.clone(),
        _ => return,
    };

    let Some(sift) = state.input.at::<SiftLayer>(r) else {
        return;
    };
    let pane = sift.snap.pane();
    let Some((bid, from)) = sift.snap.selections().map(|(bid, s)| (bid, s.clone())) else {
        return;
    };

    #[cfg(test)]
    {
        state.sift_scans += 1;
    }
    match sift_within(state.buffers.get(bid).text(), from.clone(), &pattern) {
        Ok(new_state) => {
            let text = new_state.text().clone();
            let sifted = new_state.into_selections();
            state
                .panes
                .entries
                .seeded_mut(pane, bid)
                .set_selections(sifted.clone(), &text);
            if let Some(sift) = state.input.at_mut::<SiftLayer>(r) {
                sift.preview = Some(SiftPreview {
                    pattern,
                    from,
                    sifted,
                });
            }
        }
        // Invalid regex in progress, or nothing matched: restore originals.
        Err(SiftFailure::Invalid | SiftFailure::NoMatch) => restore_sift_snapshot(state, r),
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
fn restore_sift_snapshot(state: &mut EditorState, r: LayerRef) {
    let Some(sift) = state.input.at_mut::<SiftLayer>(r) else {
        return;
    };
    sift.preview = None;
    sift.snap.restore(&mut state.panes.entries, &state.buffers);
}
