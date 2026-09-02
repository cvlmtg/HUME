use std::borrow::Cow;

use crate::editor::registry::{
    CommandRegistry, MappableCommand, SelectionBody, SelectionTracking, StructuralBody,
};
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan};

/// One row of the structural text-object / navigation family: the kind, its
/// `m i` / `m a` third-level key, the four command names it registers, and
/// their four doc strings — static rather than templated from a shared noun,
/// since "including its delimiters" is wrong for an argument (a separator
/// comma, never brackets) and a function (its signature, not delimiters).
/// One table drives both registration (`register_structural`, below) and the
/// keymap (`keymap/defaults::build_text_object_trie`) — a kind added here
/// needs no change anywhere else. Doc wording mirrors
/// `user-manual/docs/builtin-commands.md`'s rows for these commands — update
/// both together.
pub(in crate::editor) struct StructuralObject {
    pub(in crate::editor) kind: ObjectKind,
    pub(in crate::editor) key: char,
    pub(in crate::editor) inner: &'static str,
    pub(in crate::editor) inner_doc: &'static str,
    pub(in crate::editor) around: &'static str,
    pub(in crate::editor) around_doc: &'static str,
    pub(in crate::editor) next: &'static str,
    pub(in crate::editor) next_doc: &'static str,
    pub(in crate::editor) prev: &'static str,
    pub(in crate::editor) prev_doc: &'static str,
}

/// Keys follow Helix (`t` = type, for `class`). `a` (argument) reuses the
/// `inner-argument`/`around-argument` names the lexical scan registered
/// before this feature — see `Argument`'s doc on `StructuralBody`.
pub(in crate::editor) const STRUCTURAL_OBJECTS: &[StructuralObject] = &[
    StructuralObject {
        kind: ObjectKind::Function,
        key: 'f',
        inner: "inner-function",
        inner_doc: "Select inside a function. Requires a grammar with a `textobjects.scm`.",
        around: "around-function",
        around_doc: "Select the function including its signature (and attributes/decorators). \
                      Requires a grammar with a `textobjects.scm`.",
        next: "goto-next-function",
        next_doc: "Select the next function.",
        prev: "goto-prev-function",
        prev_doc: "Select the previous function.",
    },
    StructuralObject {
        kind: ObjectKind::Class,
        key: 't',
        inner: "inner-class",
        inner_doc: "Select inside a class or type. Requires a grammar with a `textobjects.scm`.",
        around: "around-class",
        around_doc: "Select the class or type including its header. Requires a grammar with a \
                      `textobjects.scm`.",
        next: "goto-next-class",
        next_doc: "Select the next class or type.",
        prev: "goto-prev-class",
        prev_doc: "Select the previous class or type.",
    },
    StructuralObject {
        kind: ObjectKind::Parameter,
        key: 'a',
        inner: "inner-argument",
        inner_doc: "Select the argument at the cursor (trimmed). Structure-aware — uses the \
                     language's `parameter` object when the grammar defines one.",
        around: "around-argument",
        around_doc: "Select the argument and its separator comma. Structure-aware — uses the \
                      language's `parameter` object when the grammar defines one.",
        next: "goto-next-argument",
        next_doc: "Select the next argument.",
        prev: "goto-prev-argument",
        prev_doc: "Select the previous argument.",
    },
    StructuralObject {
        kind: ObjectKind::Comment,
        key: 'c',
        inner: "inner-comment",
        inner_doc: "Select inside a comment. Requires a grammar with a `textobjects.scm`.",
        around: "around-comment",
        around_doc: "Select the whole comment block. Requires a grammar with a \
                      `textobjects.scm`.",
        next: "goto-next-comment",
        next_doc: "Select the next comment.",
        prev: "goto-prev-comment",
        prev_doc: "Select the previous comment.",
    },
    StructuralObject {
        kind: ObjectKind::Test,
        key: 'T',
        inner: "inner-test",
        inner_doc: "Select inside a test function's body. Requires a grammar with a \
                     `textobjects.scm`.",
        around: "around-test",
        around_doc: "Select the whole test, including its attribute or decorator. Requires a \
                      grammar with a `textobjects.scm`.",
        next: "goto-next-test",
        next_doc: "Select the next test.",
        prev: "goto-prev-test",
        prev_doc: "Select the previous test.",
    },
    StructuralObject {
        kind: ObjectKind::Entry,
        key: 'e',
        inner: "inner-entry",
        inner_doc: "Select inside an array/tuple/struct entry. Requires a grammar with a \
                     `textobjects.scm`.",
        around: "around-entry",
        around_doc: "Select an array/tuple/struct entry plus its separator comma. Requires a \
                      grammar with a `textobjects.scm`.",
        next: "goto-next-entry",
        next_doc: "Select the next array/tuple/struct entry.",
        prev: "goto-prev-entry",
        prev_doc: "Select the previous array/tuple/struct entry.",
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
                doc: Cow::Borrowed(obj.inner_doc),
                fun: SelectionBody::Structural(inner_body),
                jump: false,
                selection_tracking: SelectionTracking::Establishes,
            });
            self.register(MappableCommand::Selection {
                name: Cow::Borrowed(obj.around),
                doc: Cow::Borrowed(obj.around_doc),
                fun: SelectionBody::Structural(around_body),
                jump: false,
                selection_tracking: SelectionTracking::Establishes,
            });
            self.register(MappableCommand::Motion {
                name: Cow::Borrowed(obj.next),
                doc: Cow::Borrowed(obj.next_doc),
                fun: SelectionBody::Structural(StructuralBody::Goto {
                    kind: obj.kind,
                    dir: Direction::Forward,
                }),
                jump: true,
            });
            self.register(MappableCommand::Motion {
                name: Cow::Borrowed(obj.prev),
                doc: Cow::Borrowed(obj.prev_doc),
                fun: SelectionBody::Structural(StructuralBody::Goto {
                    kind: obj.kind,
                    dir: Direction::Backward,
                }),
                jump: true,
            });
        }
    }
}
