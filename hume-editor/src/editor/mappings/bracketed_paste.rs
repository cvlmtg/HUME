//! Terminal bracketed-paste handling (`Event::Paste`).
//!
//! Named `handle_terminal_paste` — not `handle_paste` — to stay clearly
//! distinct from the register/kill-ring `p`/`P` "paste" commands in
//! `editor::commands::paste`, which are an unrelated feature.

use super::super::input_stack::InputEvent;
use super::super::{Editor, doc_ops};
use hume_editing::text::normalize_line_endings;
use hume_ops::edit::insert_str;

impl Editor {
    // ── Terminal paste ───────────────────────────────────────────────────────

    /// Handle a whole pasted string arriving as one terminal event.
    ///
    /// Normalizes and empty-checks at the terminal boundary, then hands off
    /// to the same stack walk keys use — each layer states its own paste
    /// policy (§2.6 in `SPEC.md`'s input-layer-stack design). See
    /// [`Editor::apply_insert_mode_paste`] for the Insert-mode path, shared
    /// with dot-repeat replay.
    pub(in crate::editor) fn handle_terminal_paste(&mut self, text: String) {
        // Normalized here, at the terminal boundary, rather than left to the
        // changeset builder: this text also reaches the minibuffer (via
        // `flatten_single_line`, whose contract is already-normalized input)
        // and the insert-session replay log, neither of which is behind that
        // builder. Terminals commonly transmit CR or CRLF for a newline in a
        // bracketed paste regardless of the source's own convention.
        let text = normalize_line_endings(&text).into_owned();
        if text.is_empty() {
            return;
        }
        // Mirrors `handle_key`'s status-message dismissal, minus the
        // summary-TTL bookkeeping — a paste is a real input event but not a
        // keystroke the TTL countdown should tick against. Unconditional now
        // that dispatch itself decides whether the paste does anything (a
        // swallow, e.g. under Confirm, still dismissed the previous message
        // as a real input event).
        self.state.status_msg.take();
        self.dispatch_input(InputEvent::Paste(text));
    }

    /// Bulk-insert `text` into the focused buffer as one grouped edit — the
    /// Insert-mode paste path. Also used by dot-repeat replay so a replayed
    /// paste re-runs as one edit rather than as synthesized per-char keys
    /// (which would wrongly re-trigger auto-indent on an embedded newline).
    ///
    /// Deliberately bypasses auto-pairs, trigger-char hooks, and per-char LSP
    /// refiltering: auto-pairing pasted brackets would corrupt already-balanced
    /// text, and refiltering a completion against a pasted blob is meaningless.
    pub(in crate::editor) fn apply_insert_mode_paste(&mut self, text: &str) {
        let focused = self.state.focus.id();
        let buf = self.focused_buffer_id();
        doc_ops::apply_doc_edit_grouped(
            &mut self.state.buffers,
            &self.state.config.decorations,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            focused,
            buf,
            |b, s| insert_str(b, s, text),
        );
        self.state.dismiss_completion(&self.view);
    }

    /// Bulk-insert `text` at every selection in the focused buffer as one
    /// edit — the `Base` layer's paste policy (Normal and Extend alike).
    pub(in crate::editor) fn apply_normal_mode_paste(&mut self, text: &str) {
        let focused = self.state.focus.id();
        let buf = self.focused_buffer_id();
        doc_ops::apply_doc_edit(
            &mut self.state.buffers,
            &self.state.config.decorations,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            focused,
            buf,
            |b, s| insert_str(b, s, text),
        );
    }
}

/// Flatten already-newline-normalized text for a single-line input field
/// (the minibuffer, a picker query): drop trailing newlines, turn any
/// interior newline into a space.
pub(super) fn flatten_single_line(text: &str) -> String {
    text.trim_end_matches('\n').replace('\n', " ")
}

#[cfg(test)]
mod tests;
