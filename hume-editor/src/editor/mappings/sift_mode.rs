use super::super::Editor;
use super::super::input_stack::LayerRef;
use super::super::minibuf::MiniBufferEvent;
use hume_ops::search::compile_search_regex;
use hume_ops::selection_cmd::sift_matches_within;

impl Editor {
    // ── Sift mode (s) ─────────────────────────────────────────────────────────

    pub(super) fn handle_sift_event(&mut self, r: LayerRef, event: MiniBufferEvent) {
        match event {
            MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => {
                self.state.truncate_layers(&self.view, r);
            }
            MiniBufferEvent::Confirm(_) => {
                // Keep the selections that live preview already set. This
                // `Confirm` arm clears its own stash before truncating, so
                // teardown's `Sift` arm (which would otherwise restore it)
                // finds nothing to do.
                let pid = self.state.focus.id();
                self.state.panes.transient[pid].pre_sift_sels = None;
                // Do NOT write to the search register or clear search state —
                // sift-within is a selection op, not a search. The previous
                // search pattern and its highlights should be preserved so that
                // n/N continues to navigate the original search.
                self.state.truncate_layers(&self.view, r);
            }
            MiniBufferEvent::EmptiedByBackspace | MiniBufferEvent::BackspaceOnEmpty => {
                // Restore original selections when pattern is fully erased.
                self.restore_sift_snapshot();
            }
            MiniBufferEvent::Edited => self.update_live_sift(),
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
    pub(super) fn update_live_sift(&mut self) {
        let pattern = match self.state.input.minibuf() {
            Some(mb) if !mb.input.is_empty() => mb.input.clone(),
            _ => return,
        };

        let Some(regex) = compile_search_regex(&pattern) else {
            // Invalid regex in progress — restore originals.
            self.restore_sift_snapshot();
            return;
        };

        // Compute matches in a limited scope so the borrow on
        // pre_sift_sels is released before we need to restore.
        let pid = self.state.focus.id();
        let result = self.state.panes.transient[pid]
            .pre_sift_sels
            .as_ref()
            .and_then(|sels| sift_matches_within(self.doc().text(), sels, &regex));

        match result {
            Some(new_sels) => self.set_current_selections(new_sels),
            None => self.restore_sift_snapshot(),
        }
    }

    // ── Snapshot restore helpers ────────────────────────────────────────────────

    /// Restore selections from the sift-mode snapshot without consuming it.
    fn restore_sift_snapshot(&mut self) {
        let pid = self.state.focus.id();
        let bid = self.focused_buffer_id();
        // pane_transient and pane_state are disjoint fields — no &mut self needed.
        if let Some(sels) = self.state.panes.transient[pid].pre_sift_sels.as_ref() {
            self.state.panes.state[pid][bid].set_selections(sels.clone());
        }
    }
}
