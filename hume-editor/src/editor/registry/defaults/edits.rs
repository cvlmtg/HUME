use crate::editor::commands::cmd_delete_word_backward;
use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, MappableCommand};
use hume_ops::edit::{
    delete_char_backward, delete_char_forward, delete_selection, make_text_capitalized,
    make_text_lowercase, make_text_uppercase,
};

use super::builder::ecmd;

impl CommandRegistry {
    pub(super) fn register_edits(&mut self) {
        // ── Edit commands ─────────────────────────────────────────────────────
        super::edit!(
            self,
            "delete-char-forward",
            "Delete the character (or selection) under the cursor.",
            delete_char_forward
        );
        super::edit!(
            self,
            "delete-char-backward",
            "Delete the character before each cursor.",
            delete_char_backward
        );
        super::edit!(
            self,
            "delete-selection",
            "Delete all selections.",
            delete_selection
        );
        // `EditorCmd`, not `edit!`: needs to resolve this buffer's
        // `word-chars` (see `cmd_delete_word_backward`'s doc). Every flag
        // below matches the plain `Edit` registration it replaces: not
        // repeatable, no jump, not extendable.
        ecmd(
            "delete-word-backward",
            "Delete the word before each cursor.",
            cmd_delete_word_backward,
        )
        .reg(self);
        super::edit!(
            self,
            "make-text-lowercase",
            "Lowercase the text in each selection.",
            make_text_lowercase,
            repeatable
        );
        super::edit!(
            self,
            "make-text-uppercase",
            "Uppercase the text in each selection.",
            make_text_uppercase,
            repeatable
        );
        super::edit!(
            self,
            "make-text-capitalized",
            "Capitalize each word in every selection (Title Case).",
            make_text_capitalized,
            repeatable
        );
    }
}
