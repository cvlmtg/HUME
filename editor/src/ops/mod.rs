pub(crate) mod edit;
pub(crate) mod motion;
pub(crate) mod pair;
pub(crate) mod register;
pub(crate) mod search;
pub(crate) mod selection_cmd;
pub(crate) mod surround;
pub(crate) mod text_object;

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
/// dispatch time. Non-extendable commands accept `_mode: MotionMode` and ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionMode {
    Move,
    Extend,
}
