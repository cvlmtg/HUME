use crate::editor::commands::{cmd_visual_move_down, cmd_visual_move_up};
use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, MappableCommand};
use hume_ops::motion::{
    cmd_goto_first_line, cmd_goto_first_nonblank, cmd_goto_last_line, cmd_goto_line_end,
    cmd_goto_line_start, cmd_move_left, cmd_move_right, cmd_next_paragraph, cmd_prev_paragraph,
    cmd_select_line, cmd_select_line_backward, cmd_select_next_uppercase_word,
    cmd_select_next_uppercase_word_around, cmd_select_next_word, cmd_select_next_word_around,
    cmd_select_prev_uppercase_word, cmd_select_prev_uppercase_word_around, cmd_select_prev_word,
    cmd_select_prev_word_around,
};

use super::builder::ecmd;

impl CommandRegistry {
    pub(super) fn register_motions(&mut self) {
        // ── Character motions ─────────────────────────────────────────────────
        super::motion!(
            self,
            "move-right",
            "Move cursors one grapheme to the right.",
            cmd_move_right
        );
        super::motion!(
            self,
            "move-left",
            "Move cursors one grapheme to the left.",
            cmd_move_left
        );
        ecmd(
            "move-down",
            "Move cursors down one visual line (one buffer line with a count).",
            cmd_visual_move_down,
        )
        .extendable()
        .visual_move()
        .reg(self);
        ecmd(
            "move-up",
            "Move cursors up one visual line (one buffer line with a count).",
            cmd_visual_move_up,
        )
        .extendable()
        .visual_move()
        .reg(self);

        // ── BufferText-level goto motions ─────────────────────────────────────────
        super::motion!(
            self,
            "goto-first-line",
            "Move cursors to the first character of the buffer.",
            cmd_goto_first_line,
            jump
        );
        super::motion!(
            self,
            "goto-last-line",
            "Move cursors to the first character of the last line.",
            cmd_goto_last_line,
            jump
        );

        // ── Line-position motions ─────────────────────────────────────────────
        super::motion!(
            self,
            "goto-line-start",
            "Move cursors to the start of the line.",
            cmd_goto_line_start
        );
        super::motion!(
            self,
            "goto-line-end",
            "Move cursors to the last character on the line.",
            cmd_goto_line_end
        );
        super::motion!(
            self,
            "goto-first-nonblank",
            "Move cursors to the first non-blank character on the line.",
            cmd_goto_first_nonblank
        );

        // ── Word motions ──────────────────────────────────────────────────────
        // All four are reaching: Move mode anchors the selection on a word
        // reached by navigating away from the cursor. Not safe to replay
        // positionally — dot-repeat would advance past the intended word.
        // Each carries an `_around` twin that covers the destination word's
        // whitespace bookend in both modes — swapped in by
        // `run_native_body` when `word-selects-whitespace` is on.
        super::motion!(
            self,
            "select-next-word",
            "Select the next word.",
            cmd_select_next_word,
            cmd_select_next_word_around,
            reaching
        );
        super::motion!(
            self,
            "select-next-uppercase-word",
            "Select the next uppercase word (whitespace-delimited).",
            cmd_select_next_uppercase_word,
            cmd_select_next_uppercase_word_around,
            reaching
        );
        super::motion!(
            self,
            "select-prev-word",
            "Select the previous word.",
            cmd_select_prev_word,
            cmd_select_prev_word_around,
            reaching
        );
        super::motion!(
            self,
            "select-prev-uppercase-word",
            "Select the previous uppercase word (whitespace-delimited).",
            cmd_select_prev_uppercase_word,
            cmd_select_prev_uppercase_word_around,
            reaching
        );

        // ── Paragraph motions ─────────────────────────────────────────────────
        super::motion!(
            self,
            "next-paragraph",
            "Move cursors to the start of the next paragraph.",
            cmd_next_paragraph
        );
        super::motion!(
            self,
            "prev-paragraph",
            "Move cursors to the first empty line above the current paragraph.",
            cmd_prev_paragraph
        );

        // ── Line selection ────────────────────────────────────────────────────
        super::selection!(
            self,
            "select-line",
            "Select the full current line (forward).",
            cmd_select_line
        );
        super::selection!(
            self,
            "select-line-backward",
            "Select the full current line (backward).",
            cmd_select_line_backward
        );
    }
}
