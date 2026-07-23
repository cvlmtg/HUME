//! Terminal bracketed-paste handling (`Event::Paste`).
//!
//! Named `handle_terminal_paste` — not `handle_paste` — to stay clearly
//! distinct from the register/kill-ring `p`/`P` "paste" commands in
//! `editor::commands::edit`, which are an unrelated feature.

use std::borrow::Cow;

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
        let text = normalize_paste_newlines(&text);
        if text.is_empty() {
            return;
        }

        // Mirrors `handle_key`'s status-message dismissal, minus the
        // summary-TTL bookkeeping — a paste is a real input event but not a
        // keystroke the TTL countdown should tick against. Dismissed once we
        // know the paste isn't empty, but per-arm below the no-op guards
        // (menu/drawer interception, an all-newline paste flattening to
        // nothing) so a paste that does nothing leaves the status untouched.
        match self.state.mode() {
            Mode::Insert => {
                self.state.status_msg.take();
                self.apply_insert_mode_paste(&text);
                if let Some(session) = self.state.insert_session.as_mut() {
                    session
                        .keystrokes
                        .push(InsertInput::Paste(text.into_owned()));
                }
            }
            Mode::Normal | Mode::Extend => {
                // A menu/drawer consumes stray input without editing the
                // buffer — same treatment `handle_key`'s intercepts give a
                // stray key while one is open.
                if self.state.menu.is_some() || self.state.drawer.is_some() {
                    return;
                }
                self.state.status_msg.take();
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
                let mb = self
                    .state
                    .minibuf
                    .as_mut()
                    .expect("minibuf present in Command/Search/Select mode");
                self.state.status_msg.take();
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
        self.clear_completion_menu();
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
                    self.state.minibuf_completion = None;
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
///
/// Single pass, and a borrow (no allocation at all) on the common case of a
/// paste with no `\r` — the two-`.replace()` version always allocated twice
/// regardless of whether anything matched.
fn normalize_paste_newlines(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Flatten already-newline-normalized text for a single-line minibuffer
/// field: drop trailing newlines, turn any interior newline into a space.
fn flatten_for_minibuf(text: &str) -> String {
    text.trim_end_matches('\n').replace('\n', " ")
}

#[cfg(test)]
mod tests;
