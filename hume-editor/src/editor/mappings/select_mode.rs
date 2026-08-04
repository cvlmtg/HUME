use termina::event::KeyEvent;

use super::super::minibuf::MiniBufferEvent;
use super::super::{Editor, Mode};
use hume_ops::search::compile_search_regex;
use hume_ops::selection_cmd::select_matches_within;

impl Editor {
    // ── Select mode (s) ────────────────────────────────────────────────────────

    pub(super) fn handle_select(&mut self, key: KeyEvent) {
        let event = match self.state.minibuf.as_mut() {
            Some(mb) => mb.handle_key(key),
            None => return,
        };
        match event {
            MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => self.cancel_select(),
            MiniBufferEvent::Confirm(_) => {
                // Keep the selections that live preview already set.
                let pid = self.state.focused_pane_id;
                self.state.panes.transient[pid].pre_select_sels = None;
                // Do NOT write to the search register or clear search state —
                // select-within is a selection op, not a search. The previous
                // search pattern and its highlights should be preserved so that
                // n/N continues to navigate the original search.
                self.set_mode(Mode::Normal);
                self.close_minibuf();
            }
            MiniBufferEvent::EmptiedByBackspace | MiniBufferEvent::BackspaceOnEmpty => {
                // Restore original selections when pattern is fully erased.
                self.restore_select_snapshot();
            }
            MiniBufferEvent::Edited => self.update_live_select(),
            // Up/Down are reserved for minibuffer history — no-op in select-within.
            MiniBufferEvent::CursorMoved
            | MiniBufferEvent::Ignored
            | MiniBufferEvent::CompleteRequested { .. }
            | MiniBufferEvent::HistoryPrev
            | MiniBufferEvent::HistoryNext => {}
        }
    }

    /// Cancel select mode: restore original selections, return to Normal.
    fn cancel_select(&mut self) {
        let pid = self.state.focused_pane_id;
        if let Some(sels) = self.state.panes.transient[pid].pre_select_sels.take() {
            self.set_current_selections(sels);
        }
        // Do not clear search state — the previous search should survive a
        // cancelled select-within.
        self.set_mode(Mode::Normal);
        self.close_minibuf();
    }

    /// Recompile the regex and replace selections with matches within the
    /// original selections. Called on every keystroke in Select mode.
    pub(super) fn update_live_select(&mut self) {
        let pattern = match self.state.minibuf.as_ref() {
            Some(mb) if !mb.input.is_empty() => mb.input.clone(),
            _ => return,
        };

        let Some(regex) = compile_search_regex(&pattern) else {
            // Invalid regex in progress — restore originals.
            self.restore_select_snapshot();
            return;
        };

        // Compute matches in a limited scope so the borrow on
        // pre_select_sels is released before we need to restore.
        let pid = self.state.focused_pane_id;
        let result = self.state.panes.transient[pid]
            .pre_select_sels
            .as_ref()
            .and_then(|sels| select_matches_within(self.doc().text(), sels, &regex));

        match result {
            Some(new_sels) => self.set_current_selections(new_sels),
            None => self.restore_select_snapshot(),
        }
    }

    // ── Snapshot restore helpers ────────────────────────────────────────────────

    /// Restore selections from the select-mode snapshot without consuming it.
    fn restore_select_snapshot(&mut self) {
        let pid = self.state.focused_pane_id;
        let bid = self.focused_buffer_id();
        // pane_transient and pane_state are disjoint fields — no &mut self needed.
        if let Some(sels) = self.state.panes.transient[pid].pre_select_sels.as_ref() {
            self.state.panes.state[pid][bid].selections = sels.clone();
        }
    }
}
