//! Terminal bracketed-paste handling (`Event::Paste`).
//!
//! Named `handle_terminal_paste` — not `handle_paste` — to stay clearly
//! distinct from the register/kill-ring `p`/`P` "paste" commands in
//! `editor::commands::paste`, which are an unrelated feature.

use super::super::Editor;
use super::super::input_stack::InputEvent;
use hume_editing::text::normalize_line_endings;

impl Editor {
    // ── Terminal paste ───────────────────────────────────────────────────────

    /// Handle a whole pasted string arriving as one terminal event.
    ///
    /// Normalizes and empty-checks at the terminal boundary, then hands off
    /// to the same stack walk keys use — each layer states its own paste
    /// policy. See `Editor::apply_insert_mode_paste`
    /// (`input_stack/insert.rs`) for the Insert-mode path, shared with
    /// dot-repeat replay.
    pub(in crate::editor) fn handle_terminal_paste(&mut self, text: String) {
        // Normalized here, at the terminal boundary, rather than left to the
        // changeset builder: this text also reaches the minibuffer (via
        // `minibuf::flatten_single_line`, whose contract is already-normalized
        // input) and the insert-session replay log, neither of which is
        // behind that builder. Terminals commonly transmit CR or CRLF for a
        // newline in a bracketed paste regardless of the source's own
        // convention.
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
}
