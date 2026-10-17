//! Terminal bracketed-paste handling (`Event::Paste`).
//!
//! Named `handle_terminal_paste` (not `handle_paste`) to stay clearly
//! distinct from the register/kill-ring `p`/`P` "paste" commands in
//! `editor::commands::paste`, which are an unrelated feature.

use super::super::Editor;
use super::super::input_stack::PastedText;

impl Editor {
    // ── Terminal paste ───────────────────────────────────────────────────────

    /// Handle a whole pasted string arriving as one terminal event: the
    /// terminal's entry to [`EditorState::paste`](super::super::EditorState::paste).
    pub(in crate::editor) fn handle_terminal_paste(&mut self, text: String) {
        let Some(text) = PastedText::new(text) else {
            return;
        };
        // Mirrors `handle_key`'s status-message dismissal, minus the
        // summary-TTL bookkeeping: a paste is a real input event but not a
        // keystroke the TTL countdown should tick against. Unconditional,
        // since the paste itself decides whether it does anything (a
        // swallow, e.g. under Confirm, still dismisses the previous message
        // as a real input event).
        self.state.status_msg.take();
        self.state.paste(&self.view, text);
    }
}
