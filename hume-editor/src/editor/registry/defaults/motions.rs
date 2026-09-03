use crate::editor::commands::{cmd_visual_move_down, cmd_visual_move_up};
use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, MappableCommand, SelectionBody, SelectionTracking};
use hume_ops::motion::{
    cmd_goto_first_line, cmd_goto_first_nonblank, cmd_goto_last_line, cmd_goto_line_end,
    cmd_goto_line_start, cmd_goto_matching_pair, cmd_goto_next_paragraph, cmd_goto_prev_paragraph,
    cmd_move_left, cmd_move_right, cmd_select_line, cmd_select_line_backward,
    cmd_select_next_uppercase_word, cmd_select_next_word, cmd_select_prev_uppercase_word,
    cmd_select_prev_word,
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

        // ── Matching pairs ─────────────────────────────────────────────────────
        super::motion!(
            self,
            "goto-matching-pair",
            "Move cursors to the matching bracket or tag.",
            cmd_goto_matching_pair,
            jump
        );

        // ── Word motions ──────────────────────────────────────────────────────
        // All four are `Motion`s (`step_update_recipe` never establishes from
        // one — see its `is_motion` exclusion), but they are the one case
        // where that matters in practice: Move mode anchors the selection on
        // a word reached by navigating away from the cursor, so unlike a
        // plain motion's bare-cursor result, this one *looks* replayable and
        // isn't — replaying it positionally would advance past the intended
        // word. Each covers the destination word's whitespace bookend in both
        // modes when the buffer's `word-selects-whitespace` resolves true —
        // see `WordCtx::around`, read inside `word_select_cmd`.
        super::motion!(
            self,
            "select-next-word",
            "Select the next word.",
            cmd_select_next_word,
            word
        );
        super::motion!(
            self,
            "select-next-uppercase-word",
            "Select the next uppercase word (whitespace-delimited).",
            cmd_select_next_uppercase_word,
            word
        );
        super::motion!(
            self,
            "select-prev-word",
            "Select the previous word.",
            cmd_select_prev_word,
            word
        );
        super::motion!(
            self,
            "select-prev-uppercase-word",
            "Select the previous uppercase word (whitespace-delimited).",
            cmd_select_prev_uppercase_word,
            word
        );

        // ── Paragraph motions ─────────────────────────────────────────────────
        // Select the whole paragraph plus its trailing blank gap, if it has
        // one, like the structural `goto-next-<kind>` family selects its
        // object — the `finder` is a lexical scan (`hume_ops::motion::paragraph`)
        // rather than a tree-sitter one, so this stays a `Plain` body here
        // instead of moving to `structural.rs`.
        super::motion!(
            self,
            "goto-next-paragraph",
            "Select the next paragraph.",
            cmd_goto_next_paragraph,
            jump
        );
        super::motion!(
            self,
            "goto-prev-paragraph",
            "Select the previous paragraph.",
            cmd_goto_prev_paragraph,
            jump
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
