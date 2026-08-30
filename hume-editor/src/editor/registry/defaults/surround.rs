use crate::editor::commands::cmd_surround_add;
use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, MappableCommand, SelectionTracking};
use hume_ops::surround::{
    cmd_surround_angle, cmd_surround_backtick, cmd_surround_brace, cmd_surround_bracket,
    cmd_surround_double_quote, cmd_surround_paren, cmd_surround_single_quote,
};

use super::builder::ecmd;

impl CommandRegistry {
    pub(super) fn register_surround(&mut self) {
        // ── Surround selection ────────────────────────────────────────────
        super::selection!(
            self,
            "surround-paren",
            "Select surrounding `()` delimiters.",
            cmd_surround_paren
        );
        super::selection!(
            self,
            "surround-bracket",
            "Select surrounding `[]` delimiters.",
            cmd_surround_bracket
        );
        super::selection!(
            self,
            "surround-brace",
            "Select surrounding `{}` delimiters.",
            cmd_surround_brace
        );
        super::selection!(
            self,
            "surround-angle",
            "Select surrounding `<>` delimiters.",
            cmd_surround_angle
        );
        super::selection!(
            self,
            "surround-double-quote",
            "Select surrounding `\"` delimiters.",
            cmd_surround_double_quote
        );
        super::selection!(
            self,
            "surround-single-quote",
            "Select surrounding `'` delimiters.",
            cmd_surround_single_quote
        );
        super::selection!(
            self,
            "surround-backtick",
            "Select surrounding backtick delimiters.",
            cmd_surround_backtick
        );

        // ── Surround add ──────────────────────────────────────────────────────
        ecmd(
            "surround-add",
            "Wrap each selection with a delimiter pair. Reads the next typed character to determine the pair.",
            cmd_surround_add,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
    }
}
