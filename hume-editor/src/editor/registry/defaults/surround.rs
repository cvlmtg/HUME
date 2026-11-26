use crate::editor::commands::{cmd_surround_add, cmd_surround_select};
use std::borrow::Cow;

use crate::editor::registry::{
    CommandRegistry, MappableCommand, NativeCommand, SelectionBody, SelectionEffect,
};
use hume_ops::surround::{
    cmd_surround_angle, cmd_surround_brace, cmd_surround_bracket, cmd_surround_paren,
};

use super::builder::ecmd_pane_char;

impl CommandRegistry {
    pub(super) fn register_surround(&mut self) {
        // ── Surround selection ────────────────────────────────────────────
        super::selection!(
            self,
            "surround-paren",
            "Select surrounding `()` delimiters.",
            cmd_surround_paren,
            seek
        );
        super::selection!(
            self,
            "surround-bracket",
            "Select surrounding `[]` delimiters.",
            cmd_surround_bracket,
            seek
        );
        super::selection!(
            self,
            "surround-brace",
            "Select surrounding `{}` delimiters.",
            cmd_surround_brace,
            seek
        );
        super::selection!(
            self,
            "surround-angle",
            "Select surrounding `<>` delimiters.",
            cmd_surround_angle,
            seek
        );
        ecmd_pane_char(
            "surround-select",
            "Select the pair of a character around each selection, as two cursors: the bracket pair it opens or closes, or else two of the character on the line. Reads the next typed character. From `call!`, pass the character as an argument.",
            cmd_surround_select,
        )
        .selection_step(SelectionEffect::Establishes)
        .reg(self);

        // ── Surround add ──────────────────────────────────────────────────────
        ecmd_pane_char(
            "surround-add",
            "Wrap each selection with a delimiter pair. Reads the next typed character to determine the pair. A count adds that many delimiter pairs: a count of two with `*` wraps in `**`. From `call!`, pass the character as an argument.",
            cmd_surround_add,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
    }
}
