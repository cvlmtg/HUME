//! Terminal bracketed-paste handling (`Event::Paste`).
//!
//! Named `handle_terminal_paste` — not `handle_paste` — to stay clearly
//! distinct from the register/kill-ring `p`/`P` "paste" commands in
//! `editor::commands::edit`, which are an unrelated feature.

use super::super::minibuf::history::{HistoryKind, HistoryStore};
use super::super::replay::InsertInput;
use super::super::{Editor, Mode, doc_ops};
use crate::ops::edit::insert_str;

impl Editor {
    // ── Terminal paste ───────────────────────────────────────────────────────

    /// Handle a whole pasted string arriving as one terminal event.
    ///
    /// Dispatches per mode so a paste is one edit (one undo step, one
    /// render) instead of running the full per-key pipeline once per
    /// character. See [`Editor::apply_insert_mode_paste`] for the Insert-mode
    /// path, shared with dot-repeat replay.
    pub(crate) fn handle_terminal_paste(&mut self, text: String) {
        // Mirrors `handle_key`'s status-message dismissal, minus the
        // summary-TTL bookkeeping — a paste is a real input event but not a
        // keystroke the TTL countdown should tick against.
        self.state.status_msg.take();

        let text = normalize_paste_newlines(&text);
        if text.is_empty() {
            return;
        }

        match self.state.mode() {
            Mode::Insert => {
                self.apply_insert_mode_paste(&text);
                if let Some(session) = self.state.insert_session.as_mut() {
                    session.keystrokes.push(InsertInput::Paste(text));
                }
            }
            Mode::Normal | Mode::Extend => {
                // A menu/drawer consumes stray input without editing the
                // buffer — same treatment `handle_key`'s intercepts give a
                // stray key while one is open.
                if self.state.menu.is_some() || self.state.drawer.is_some() {
                    return;
                }
                let focused = self.state.focused_pane_id;
                let buf = self.focused_buffer_id();
                doc_ops::apply_doc_edit(
                    &mut self.state.buffers,
                    &self.state.decorations,
                    &mut self.state.panes.state,
                    focused,
                    buf,
                    |b, s| insert_str(b, s, &text),
                );
            }
            Mode::Command | Mode::Search | Mode::Select => {
                let flattened = flatten_for_minibuf(&text);
                if flattened.is_empty() {
                    return;
                }
                let Some(mb) = self.state.minibuf.as_mut() else {
                    return;
                };
                mb.insert_str(&flattened);
                self.on_minibuf_paste_edited();
            }
        }
    }

    /// Bulk-insert `text` into the focused buffer as one grouped edit — the
    /// Insert-mode paste path. Also used by dot-repeat replay so a replayed
    /// paste re-runs as one edit rather than as synthesized per-char keys
    /// (which would wrongly re-trigger auto-indent on an embedded newline).
    ///
    /// Deliberately bypasses auto-pairs, trigger-char hooks, and per-char LSP
    /// refiltering: auto-pairing pasted brackets would corrupt already-balanced
    /// text, and refiltering a completion against a pasted blob is meaningless.
    pub(crate) fn apply_insert_mode_paste(&mut self, text: &str) {
        let focused = self.state.focused_pane_id;
        let buf = self.focused_buffer_id();
        doc_ops::apply_doc_edit_grouped(
            &mut self.state.buffers,
            &self.state.decorations,
            &mut self.state.panes.state,
            focused,
            buf,
            |b, s| insert_str(b, s, text),
        );
        self.clear_lsp_completion();
        // Typing (pasting) real content cancels the "nothing typed since
        // Enter" state — same rule the printable-char branch of
        // `handle_insert` applies.
        self.state.autoindent_pending = false;
    }

    /// Runs the same follow-up each mode performs on `MiniBufferEvent::Edited`
    /// (completion/history invalidation, live search/select refresh) after a
    /// paste has already been inserted into the minibuffer directly.
    fn on_minibuf_paste_edited(&mut self) {
        match self.state.mode() {
            Mode::Command => {
                // A `(prompt! …)` session applies plain edits with no further
                // follow-up — mirrors `handle_steel_prompt_event`'s Edited arm.
                if self.state.steel_prompt_callback.is_none() {
                    self.state.completion = None;
                    self.state
                        .history
                        .get_mut(HistoryKind::Command)
                        .demote_to_scratch();
                }
            }
            Mode::Search => {
                if let Some(k) = self
                    .state
                    .minibuf
                    .as_ref()
                    .and_then(|m| HistoryStore::kind_for_prompt(&m.prompt))
                {
                    self.state.history.get_mut(k).demote_to_scratch();
                }
                self.update_live_search();
            }
            Mode::Select => self.update_live_select(),
            Mode::Normal | Mode::Extend | Mode::Insert => {}
        }
    }
}

/// Normalize terminal line-ending conventions in pasted text: `\r\n` and lone
/// `\r` both become `\n`. Terminals commonly transmit CR (or CRLF) for
/// newlines in a bracketed paste regardless of the source file's own
/// convention.
fn normalize_paste_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Flatten already-newline-normalized text for a single-line minibuffer
/// field: drop trailing newlines, turn any interior newline into a space.
fn flatten_for_minibuf(text: &str) -> String {
    text.trim_end_matches('\n').replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::{flatten_for_minibuf, normalize_paste_newlines};

    #[test]
    fn normalizes_crlf() {
        assert_eq!(normalize_paste_newlines("a\r\nb"), "a\nb");
    }

    #[test]
    fn normalizes_lone_cr() {
        assert_eq!(normalize_paste_newlines("a\rb"), "a\nb");
    }

    #[test]
    fn normalize_is_noop_on_lf() {
        assert_eq!(normalize_paste_newlines("a\nb"), "a\nb");
    }

    #[test]
    fn flatten_drops_trailing_newline() {
        assert_eq!(flatten_for_minibuf("foo\n"), "foo");
    }

    #[test]
    fn flatten_turns_interior_newline_into_space() {
        assert_eq!(flatten_for_minibuf("foo\nbar"), "foo bar");
    }

    #[test]
    fn flatten_drops_multiple_trailing_newlines() {
        assert_eq!(flatten_for_minibuf("foo\n\n"), "foo");
    }
}
