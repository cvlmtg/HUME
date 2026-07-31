use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, MappableCommand};
use hume_ops::edit::{
    delete_char_backward, delete_char_forward, delete_selection, delete_word_backward,
    make_text_capitalized, make_text_lowercase, make_text_uppercase,
};

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
        super::edit!(
            self,
            "delete-word-backward",
            "Delete the word before each cursor.",
            delete_word_backward
        );
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
