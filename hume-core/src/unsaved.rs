//! Consent to drop a buffer's unsaved changes, shared by the scripting
//! builtin that closes a buffer and the editor actions that drop text.

/// Whether a close, reload or quit may drop unsaved changes: the consent of
/// the user (a `!`, a confirm answer) or of a script (`close-buffer!`'s
/// `#:discard-unsaved`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnUnsaved {
    Refuse,
    Discard,
}

impl OnUnsaved {
    /// What `force` consents to: a typed command's `!`, or
    /// `#:discard-unsaved #t`.
    pub fn from_force(force: bool) -> Self {
        if force { Self::Discard } else { Self::Refuse }
    }
}
