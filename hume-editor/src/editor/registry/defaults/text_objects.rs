use crate::editor::commands::cmd_visual_select_word_nearest_on_line;
use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, MappableCommand, SelectionBody, SelectionTracking};
use hume_ops::text_object::{
    cmd_around_angle, cmd_around_backtick, cmd_around_brace, cmd_around_bracket,
    cmd_around_double_quote, cmd_around_line, cmd_around_paren, cmd_around_single_quote,
    cmd_around_uppercase_word, cmd_around_word, cmd_inner_angle, cmd_inner_backtick,
    cmd_inner_brace, cmd_inner_bracket, cmd_inner_double_quote, cmd_inner_line, cmd_inner_paren,
    cmd_inner_single_quote, cmd_inner_uppercase_word, cmd_inner_word, cmd_select_uppercase_word,
    cmd_select_word,
};

use super::builder::ecmd;

impl CommandRegistry {
    pub(super) fn register_text_objects(&mut self) {
        // ── Text objects — line ───────────────────────────────────────────────
        super::selection!(
            self,
            "inner-line",
            "Select inner line content (excluding the newline).",
            cmd_inner_line
        );
        super::selection!(
            self,
            "around-line",
            "Select the line including its newline.",
            cmd_around_line
        );

        // ── Text objects — word ───────────────────────────────────────────────
        super::selection!(
            self,
            "inner-word",
            "Select inner word.",
            cmd_inner_word,
            word
        );
        // `EditorCmd`, not `selection!`: needs a `RowMap` for wrap-aware
        // nearest-word placement (see `visual_move.rs::cmd_visual_select_word_nearest_on_line`).
        // `.establishes_selection()`: same in-place establishing semantics
        // as `select-word` (`mm`, a plain `Selection`) — replayable on its
        // own from a fresh cursor.
        ecmd(
            "select-word-nearest-on-line",
            "Select the word under the cursor, or the nearest word on the same visual line \
             when on whitespace; span follows word-selects-whitespace.",
            cmd_visual_select_word_nearest_on_line,
        )
        .extendable()
        .establishes_selection()
        .reg(self);
        super::selection!(
            self,
            "around-word",
            "Select word plus one adjacent whitespace run.",
            cmd_around_word,
            word
        );
        super::selection!(
            self,
            "inner-uppercase-word",
            "Select inner uppercase word (whitespace-delimited).",
            cmd_inner_uppercase_word,
            word
        );
        super::selection!(
            self,
            "around-uppercase-word",
            "Select uppercase word plus one adjacent whitespace run.",
            cmd_around_uppercase_word,
            word
        );
        // `mm`/`MM`: select the word/WORD under the cursor. Unlike
        // `inner-word`/`around-word` (`miw`/`maw`, never flag-affected), these
        // cover the destination word's whitespace bookend when
        // `word-selects-whitespace` resolves true — see `WordCtx::around`,
        // read inside `cmd_select_word`/`cmd_select_uppercase_word`.
        super::selection!(
            self,
            "select-word",
            "Select the word under the cursor.",
            cmd_select_word,
            word
        );
        super::selection!(
            self,
            "select-uppercase-word",
            "Select the uppercase word (WORD) under the cursor.",
            cmd_select_uppercase_word,
            word
        );

        // ── Text objects — brackets ───────────────────────────────────────────
        super::selection!(
            self,
            "inner-paren",
            "Select content inside the nearest `()`.",
            cmd_inner_paren
        );
        super::selection!(
            self,
            "around-paren",
            "Select content including the nearest `()`.",
            cmd_around_paren
        );
        super::selection!(
            self,
            "inner-bracket",
            "Select content inside the nearest `[]`.",
            cmd_inner_bracket
        );
        super::selection!(
            self,
            "around-bracket",
            "Select content including the nearest `[]`.",
            cmd_around_bracket
        );
        super::selection!(
            self,
            "inner-brace",
            "Select content inside the nearest `{}`.",
            cmd_inner_brace
        );
        super::selection!(
            self,
            "around-brace",
            "Select content including the nearest `{}`.",
            cmd_around_brace
        );
        super::selection!(
            self,
            "inner-angle",
            "Select content inside the nearest `<>`.",
            cmd_inner_angle
        );
        super::selection!(
            self,
            "around-angle",
            "Select content including the nearest `<>`.",
            cmd_around_angle
        );

        // ── Text objects — quotes ─────────────────────────────────────────────
        super::selection!(
            self,
            "inner-double-quote",
            "Select content inside the nearest `\"`.",
            cmd_inner_double_quote
        );
        super::selection!(
            self,
            "around-double-quote",
            "Select content including the nearest `\"`.",
            cmd_around_double_quote
        );
        super::selection!(
            self,
            "inner-single-quote",
            "Select content inside the nearest `'`.",
            cmd_inner_single_quote
        );
        super::selection!(
            self,
            "around-single-quote",
            "Select content including the nearest `'`.",
            cmd_around_single_quote
        );
        super::selection!(
            self,
            "inner-backtick",
            "Select content inside the nearest backtick pair.",
            cmd_inner_backtick
        );
        super::selection!(
            self,
            "around-backtick",
            "Select content including the nearest backtick pair.",
            cmd_around_backtick
        );

        // `inner-argument` / `around-argument` register from
        // `register_structural` (structural.rs): structure-aware, with this
        // module's lexical scan as their no-grammar/no-match fallback.
    }
}
