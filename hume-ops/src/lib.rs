//! HUME's named editing commands, as pure functions of a buffer text and
//! its selections.
//!
//! A motion, text object or selection command takes an
//! `hume_editing::state::EditState`, plus its own parameters (a count, a
//! [`MotionMode`], a character), and returns the next one. An edit returns
//! an `hume_editing::edit::Edited`, which also carries the changeset it made.
//! The editor binds keys to these functions by name. This crate does not
//! depend on `hume-editor`, so no command can see which key ran it.
//!
//! - [`motion`], [`text_object`] and [`selection_cmd`] move and reshape
//!   selections.
//! - [`edit`] and [`surround`] change text; [`auto_pairs`] decides what
//!   typing a bracket or quote inserts.
//! - [`search`] and [`pair`] scan text for regex matches and delimiters.
//! - [`register`] holds yanked text and recorded macros.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

pub mod auto_pairs;
pub mod edit;
pub mod motion;
pub mod pair;
pub mod register;
pub mod search;
pub mod selection_cmd;
pub mod surround;
mod tag;
pub mod text_object;
mod word_unit;

/// Controls how a motion updates the selection's anchor and head.
///
/// | Mode | Anchor | Head | Usage |
/// |------|--------|------|-------|
/// | `Move`   | `new_head` | `new_head` | Plain cursor move: anchor re-set to head |
/// | `Extend` | `old_anchor` | `new_head` | Grow selection: keep existing anchor |
///
/// `Move` always produces a collapsed single-character selection (anchor == head).
/// `Extend` keeps the existing anchor, only moving the head.
///
/// All Motion, Selection, and EditorCmd functions receive a `MotionMode` at
/// dispatch time. Most motion and text-object commands branch on it (word-select
/// and text-object commands use [`Extend`](MotionMode::Extend) to union new
/// ranges with the existing selection rather than replacing it). Commands with
/// no extend semantics (e.g. surround, flip, collapse) accept `_mode` and
/// ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionMode {
    Move,
    Extend,
}

/// Where a motion that stops at the end of a line lands. The caller decides
/// from the mode: a Normal or Extend cursor sits on a character, while an
/// Insert cursor marks where typed text goes, which at the end of a line is
/// its line break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnd {
    /// The line's last cluster, or its `\n` on an empty line.
    LastCluster,
    /// The line's `\n`.
    LineBreak,
}

/// Context for the word-family motions and text objects (`w`/`W`/`b`/`B`,
/// `mm`/`MM`, `miw`/`maw`), resolved once by the caller from buffer settings.
///
/// This family needs more than the shared `(EditState, count, MotionMode)`
/// shape: `hume-ops` cannot depend on `hume-editor`'s settings, so the
/// caller resolves `whitespace`/`chars` and passes them in, the same way
/// `tab_width`/`TabStyle` are resolved and passed to `align_selections`/
/// `insert_tab`. Keeping this as its own struct (rather than widening every
/// `hume-ops` selection command's signature) means the large majority of
/// registrations (the word-unrelated ones) never carry data they ignore.
#[derive(Debug, Clone, Copy)]
pub struct WordCtx<'a> {
    pub mode: MotionMode,
    /// Effective `word-selects-whitespace`: which side's whitespace the
    /// destination word's bookend prefers, or `None` for the bare word.
    /// `miw`/`miW` are fixed by name and ignore this field, the same way a
    /// non-extendable command ignores `mode`.
    pub whitespace: Option<WhitespaceSide>,
    pub chars: hume_editing::word::WordChars<'a>,
}

/// The side of a word whose whitespace run its bookend takes first. The
/// other side is the fallback when the preferred one has no run, and a run
/// that is a line's indentation is never taken from either side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhitespaceSide {
    Before,
    After,
}

impl<'a> WordCtx<'a> {
    #[cfg(any(test, feature = "test-util"))]
    pub fn bare(mode: MotionMode) -> Self {
        Self::with_whitespace(mode, None)
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn before(mode: MotionMode) -> Self {
        Self::with_whitespace(mode, Some(WhitespaceSide::Before))
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn after(mode: MotionMode) -> Self {
        Self::with_whitespace(mode, Some(WhitespaceSide::After))
    }

    #[cfg(any(test, feature = "test-util"))]
    fn with_whitespace(mode: MotionMode, whitespace: Option<WhitespaceSide>) -> Self {
        Self {
            mode,
            whitespace,
            chars: hume_editing::word::WordChars::default(),
        }
    }

    /// Swap in a non-default `word-chars` value, keeping `mode`/`whitespace`,
    /// for tests that need a configured char without hand-writing the whole
    /// struct literal.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_chars(self, chars: hume_editing::word::WordChars<'a>) -> Self {
        Self { chars, ..self }
    }
}
