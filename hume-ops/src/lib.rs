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

// ── MotionMode ────────────────────────────────────────────────────────────────

/// Controls how a motion updates the selection's anchor and head.
///
/// | Mode | Anchor | Head | Usage |
/// |------|--------|------|-------|
/// | `Move`   | `new_head` | `new_head` | Plain cursor move — anchor re-set to head |
/// | `Extend` | `old_anchor` | `new_head` | Grow selection — keep existing anchor |
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

// ── WordCtx ──────────────────────────────────────────────────────────────────

/// Context for the word-family motions and text objects (`w`/`W`/`b`/`B`,
/// `mm`/`MM`, `miw`/`maw`), resolved once by the caller from buffer settings.
///
/// This family needs more than the shared `(text, sels, count, MotionMode)`
/// shape — `hume-ops` cannot depend on `hume-editor`'s settings, so the
/// caller resolves `around`/`chars` and passes them in, the same way
/// `tab_width`/`TabStyle` are resolved and passed to `align_selections`/
/// `insert_tab`. Keeping this as its own struct (rather than widening every
/// `hume-ops` selection command's signature) means the ~28 word-unrelated
/// commands never carry data they ignore.
#[derive(Debug, Clone, Copy)]
pub struct WordCtx<'a> {
    pub mode: MotionMode,
    /// Effective `word-selects-whitespace`: whether the destination word's
    /// whitespace bookend is covered. `miw`/`maw`/`miW`/`maW` are fixed by
    /// name and ignore this field, the same way a non-extendable command
    /// ignores `mode`.
    pub around: bool,
    pub chars: hume_editing::word::WordChars<'a>,
}

impl<'a> WordCtx<'a> {
    #[cfg(test)]
    pub fn bare(mode: MotionMode) -> Self {
        Self {
            mode,
            around: false,
            chars: hume_editing::word::WordChars::default(),
        }
    }

    #[cfg(test)]
    pub fn around(mode: MotionMode) -> Self {
        Self {
            mode,
            around: true,
            chars: hume_editing::word::WordChars::default(),
        }
    }
}
