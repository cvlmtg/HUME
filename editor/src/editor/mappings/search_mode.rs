use std::sync::Arc;

use crossterm::event::KeyEvent;

use super::super::commands::search_sel;
use super::super::minibuf::MiniBufferEvent;
use super::super::{search_ops, Editor, Mode, SearchDirection};
use crate::core::jump_list::JumpEntry;
use crate::core::minibuf_history::{HistoryDir, HistoryStore};
use crate::core::search_state::SearchPattern;
use crate::ops::search::{compile_search_regex, find_next_match};

impl Editor {
    // ── Search mode ───────────────────────────────────────────────────────────

    pub(super) fn handle_search(&mut self, key: KeyEvent) {
        let event = match self.minibuf.as_mut() {
            Some(mb) => mb.handle_key(key),
            None => return,
        };
        match event {
            MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => self.cancel_search(),
            MiniBufferEvent::Confirm(pattern) => {
                // Record into the correct search ring before closing the minibuf.
                let kind = self
                    .minibuf
                    .as_ref()
                    .and_then(|m| HistoryStore::kind_for_prompt(m.prompt));
                if let Some(k) = kind {
                    self.history.get_mut(k).push(pattern.clone());
                }
                // Persist pattern in 's' register for future n/N.
                self.registers.write_text(crate::ops::register::SEARCH_REGISTER, vec![pattern]);
                // Record the pre-search position in the jump list before
                // discarding it — the search moved the cursor to the match.
                let pid = self.focused_pane_id;
                if let Some(sels) = self.pane_transient[pid].pre_search_sels.take() {
                    let bid = self.focused_buffer_id();
                    let entry = JumpEntry::new(sels, self.doc().text(), bid);
                    self.pane_jumps[self.focused_pane_id].push(entry);
                }
                // search_pattern stays alive on the buffer for immediate n/N without recompile.
                // set_mode does not touch search state, so it is safe to call here.
                self.set_mode(Mode::Normal);
                self.close_minibuf();
            }
            MiniBufferEvent::EmptiedByBackspace => {
                // First Backspace cleared the last character — restore position but
                // stay in Search mode. A second Backspace (BackspaceOnEmpty) dismisses.
                self.restore_search_snapshot();
                let bid = self.focused_buffer_id();
                search_ops::clear_buffer_search(&mut self.buffers, &mut self.pane_state, bid);
            }
            MiniBufferEvent::BackspaceOnEmpty => {
                // Input already empty — user pressed Backspace a second time to dismiss.
                self.cancel_search();
            }
            MiniBufferEvent::Edited => {
                if let Some(k) = self
                    .minibuf
                    .as_ref()
                    .and_then(|m| HistoryStore::kind_for_prompt(m.prompt))
                {
                    self.history.get_mut(k).demote_to_scratch();
                }
                self.update_live_search();
            }
            MiniBufferEvent::HistoryPrev => {
                let Some(prompt) = self.minibuf.as_ref().map(|m| m.prompt) else {
                    return;
                };
                let Some(kind) = HistoryStore::kind_for_prompt(prompt) else {
                    return;
                };
                self.recall_history(kind, HistoryDir::Prev);
                self.update_live_search();
            }
            MiniBufferEvent::HistoryNext => {
                let Some(prompt) = self.minibuf.as_ref().map(|m| m.prompt) else {
                    return;
                };
                let Some(kind) = HistoryStore::kind_for_prompt(prompt) else {
                    return;
                };
                self.recall_history(kind, HistoryDir::Next);
                self.update_live_search();
            }
            MiniBufferEvent::CursorMoved
            | MiniBufferEvent::Ignored
            | MiniBufferEvent::CompleteRequested { .. } => {}
        }
    }

    /// Cancel search: restore pre-search position, clear all search state, return to Normal.
    fn cancel_search(&mut self) {
        let pid = self.focused_pane_id;
        if let Some(sels) = self.pane_transient[pid].pre_search_sels.take() {
            self.set_current_selections(sels);
        }
        let bid = self.focused_buffer_id();
        search_ops::clear_buffer_search(&mut self.buffers, &mut self.pane_state, bid);
        self.mode = Mode::Normal;
        self.close_minibuf();
    }

    /// Recompile the regex from the current mini-buffer input and jump to the
    /// first match from the pre-search position.
    ///
    /// Called on every keystroke while in Search mode.
    fn update_live_search(&mut self) {
        let pattern = match self.minibuf.as_ref() {
            Some(mb) if !mb.input.is_empty() => mb.input.clone(),
            _ => return,
        };

        let Some(regex) = compile_search_regex(&pattern) else {
            // Invalid regex in progress — clear pattern so highlights disappear.
            let bid = self.focused_buffer_id();
            search_ops::clear_buffer_search(&mut self.buffers, &mut self.pane_state, bid);
            return;
        };

        let direction = self.search.direction;
        let pid = self.focused_pane_id;

        // Start from the original pre-search position (not the current position),
        // so each additional character refines from the same anchor point.
        let from_char = {
            let pt = &self.pane_transient[pid];
            match &pt.pre_search_sels {
                Some(sels) => {
                    let buf = self.doc().text();
                    let primary = sels.primary();
                    match direction {
                        SearchDirection::Forward => primary.start(),
                        SearchDirection::Backward => primary.end_inclusive(buf),
                    }
                }
                None => 0,
            }
        };

        match find_next_match(self.doc().text(), &regex, from_char, direction) {
            Some((start, end_incl, _wrapped)) => {
                let anchor = if self.pane_transient[pid].search_extend {
                    // Extend from the original anchor.
                    Some(
                        self.pane_transient[pid]
                            .pre_search_sels
                            .as_ref()
                            .map(|s| s.primary().anchor)
                            .unwrap_or(start),
                    )
                } else {
                    None
                };
                self.set_primary_selection(search_sel(start, end_incl, anchor, direction));
            }
            None => {
                // No match — restore position to pre-search.
                self.restore_search_snapshot();
            }
        }

        let bid = self.focused_buffer_id();
        self.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
            regex: Arc::new(regex),
            pattern_str: pattern,
        });
    }

    // ── Snapshot restore helpers ────────────────────────────────────────────────

    /// Restore selections from the search-mode snapshot without consuming it.
    fn restore_search_snapshot(&mut self) {
        let pid = self.focused_pane_id;
        let bid = self.focused_buffer_id();
        // pane_transient and pane_state are disjoint fields — no &mut self needed.
        if let Some(sels) = self.pane_transient[pid].pre_search_sels.as_ref() {
            self.pane_state[pid][bid].selections = sels.clone();
        }
    }
}
