use std::borrow::Cow;

use crate::editor::registry::{
    CommandRegistry, MappableCommand, SelectionBody, SelectionTracking, StructuralBody,
};
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan};

/// One row of the structural text-object / navigation family: the kind, its
/// `m i` / `m a` third-level key, the noun its docs are worded around, and
/// the four command names it registers. One table drives both registration
/// (`register_structural`, below) and the keymap
/// (`keymap/defaults::build_text_object_trie`) — a kind added here needs no
/// change anywhere else.
pub(in crate::editor) struct StructuralObject {
    pub(in crate::editor) kind: ObjectKind,
    pub(in crate::editor) key: char,
    pub(in crate::editor) noun: &'static str,
    pub(in crate::editor) inner: &'static str,
    pub(in crate::editor) around: &'static str,
    pub(in crate::editor) next: &'static str,
    pub(in crate::editor) prev: &'static str,
}

/// Keys follow Helix (`t` = type, for `class`). `a` (argument) reuses the
/// `inner-argument`/`around-argument` names the lexical scan registered
/// before this feature — see `Argument`'s doc on `StructuralBody`.
pub(in crate::editor) const STRUCTURAL_OBJECTS: &[StructuralObject] = &[
    StructuralObject {
        kind: ObjectKind::Function,
        key: 'f',
        noun: "function",
        inner: "inner-function",
        around: "around-function",
        next: "goto-next-function",
        prev: "goto-prev-function",
    },
    StructuralObject {
        kind: ObjectKind::Class,
        key: 't',
        noun: "class",
        inner: "inner-class",
        around: "around-class",
        next: "goto-next-class",
        prev: "goto-prev-class",
    },
    StructuralObject {
        kind: ObjectKind::Parameter,
        key: 'a',
        noun: "argument",
        inner: "inner-argument",
        around: "around-argument",
        next: "goto-next-argument",
        prev: "goto-prev-argument",
    },
    StructuralObject {
        kind: ObjectKind::Comment,
        key: 'c',
        noun: "comment",
        inner: "inner-comment",
        around: "around-comment",
        next: "goto-next-comment",
        prev: "goto-prev-comment",
    },
    StructuralObject {
        kind: ObjectKind::Test,
        key: 'T',
        noun: "test",
        inner: "inner-test",
        around: "around-test",
        next: "goto-next-test",
        prev: "goto-prev-test",
    },
    StructuralObject {
        kind: ObjectKind::Entry,
        key: 'e',
        noun: "entry",
        inner: "inner-entry",
        around: "around-entry",
        next: "goto-next-entry",
        prev: "goto-prev-entry",
    },
];

impl CommandRegistry {
    /// Register the six structural kinds' four commands each (24 names; the
    /// two `Parameter` names are the pre-existing `inner-argument`/
    /// `around-argument`, now `Argument`-bodied instead of lexical-only).
    ///
    /// `Select`/`Argument` register as `Selection` (`Establishes` — each
    /// replayable on its own from a fresh cursor, same as `select-line`/
    /// `ms(`); `Goto` registers as `Motion` (`jump: true` — a goto records a
    /// jump-list entry, and `Motion` always carries `SelectionTracking::
    /// Extends`, so a Move-mode press is not replayed by `.` while an Extend
    /// step is).
    pub(super) fn register_structural(&mut self) {
        for obj in STRUCTURAL_OBJECTS {
            let (inner_body, around_body) = if obj.kind == ObjectKind::Parameter {
                (
                    StructuralBody::Argument { around: false },
                    StructuralBody::Argument { around: true },
                )
            } else {
                (
                    StructuralBody::Select {
                        kind: obj.kind,
                        span: ObjectSpan::Inside,
                    },
                    StructuralBody::Select {
                        kind: obj.kind,
                        span: ObjectSpan::Around,
                    },
                )
            };

            self.register(MappableCommand::Selection {
                name: Cow::Borrowed(obj.inner),
                doc: Cow::Owned(format!(
                    "Select the inside of the {} at the cursor.",
                    obj.noun
                )),
                fun: SelectionBody::Structural(inner_body),
                jump: false,
                selection_tracking: SelectionTracking::Establishes,
            });
            self.register(MappableCommand::Selection {
                name: Cow::Borrowed(obj.around),
                doc: Cow::Owned(format!(
                    "Select the {} at the cursor, including its delimiters.",
                    obj.noun
                )),
                fun: SelectionBody::Structural(around_body),
                jump: false,
                selection_tracking: SelectionTracking::Establishes,
            });
            self.register(MappableCommand::Motion {
                name: Cow::Borrowed(obj.next),
                doc: Cow::Owned(format!("Select the next {}.", obj.noun)),
                fun: SelectionBody::Structural(StructuralBody::Goto {
                    kind: obj.kind,
                    dir: Direction::Forward,
                }),
                jump: true,
            });
            self.register(MappableCommand::Motion {
                name: Cow::Borrowed(obj.prev),
                doc: Cow::Owned(format!("Select the previous {}.", obj.noun)),
                fun: SelectionBody::Structural(StructuralBody::Goto {
                    kind: obj.kind,
                    dir: Direction::Backward,
                }),
                jump: true,
            });
        }
    }
}
