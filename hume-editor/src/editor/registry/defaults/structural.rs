use std::borrow::Cow;

use crate::editor::registry::{
    CommandRegistry, MappableCommand, SelectionBody, SelectionTracking, StructuralBody,
};
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan};

/// One row of the structural text-object / navigation family: the kind, its
/// `m i` / `m a` third-level key, the four command names it registers, and
/// the `inner`/`around` pair's doc strings, static rather than templated
/// from `noun` — "including its delimiters" is wrong for an argument (a
/// separator comma, never brackets) and a function (its signature, not
/// delimiters). `next`/`prev`'s docs carry no such per-kind irregularity
/// ("Select the next/previous `<noun>`." is exact for all six), so
/// `register_structural` derives them from `noun` instead of a fifth and
/// sixth static string. One table drives both registration
/// (`register_structural`, below) and the keymap
/// (`keymap/defaults::build_text_object_trie`) — a kind added here needs no
/// change anywhere else. Doc wording mirrors
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
    pub(in crate::editor) prev: &'static str,
    /// The kind's name as it reads in "Select the next/previous `<noun>`." —
    /// e.g. `"function"`, `"class or type"`.
    pub(in crate::editor) noun: &'static str,
}

/// Keys mostly follow Helix (`t` = type, for `class`). `a` (argument) reuses
/// the `inner-argument`/`around-argument` names the lexical scan registered
/// before this feature — see `Argument`'s doc on `StructuralBody`.
///
/// `test` and `entry` deliberately diverge from Helix's own letters (`T` and
/// `e`) to fit `keymap/defaults::build_goto_trie`'s `g <key>`/`g <KEY>`
/// scheme, which derives the "previous" bind by uppercasing `key` — that
/// requires every `key` here to be lowercase (enforced by a
/// `debug_assert!` in that function) and every uppercased form to be
/// distinct. Helix's `T` has no such lowercase form, and `e` collides with
/// HUME's native `goto-last-line`. `u` ("unit test") and `v` ("value", for
/// `entry`) are free in both this and the `g` trie.
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
        prev: "goto-prev-function",
        noun: "function",
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
        prev: "goto-prev-class",
        noun: "class or type",
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
        prev: "goto-prev-argument",
        noun: "argument",
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
        prev: "goto-prev-comment",
        noun: "comment",
    },
    StructuralObject {
        kind: ObjectKind::Test,
        key: 'u',
        inner: "inner-test",
        inner_doc: "Select inside a unit test function's body. Requires a grammar with a \
                     `textobjects.scm`.",
        around: "around-test",
        around_doc: "Select the whole unit test, including its attribute or decorator. Requires \
                      a grammar with a `textobjects.scm`.",
        next: "goto-next-test",
        prev: "goto-prev-test",
        noun: "unit test",
    },
    StructuralObject {
        // `ObjectKind::Entry` names the tree-sitter capture half
        // (`@entry.inside`/`@entry.around`) in upstream Helix-format
        // `textobjects.scm` files — not ours to rename. Only the
        // user-facing command names, key, and doc strings below use
        // "value" instead.
        kind: ObjectKind::Entry,
        key: 'v',
        inner: "inner-value",
        inner_doc: "Select inside an array/tuple/struct value. Requires a grammar with a \
                     `textobjects.scm`.",
        around: "around-value",
        around_doc: "Select an array/tuple/struct value plus its separator comma. Requires a \
                      grammar with a `textobjects.scm`.",
        next: "goto-next-value",
        prev: "goto-prev-value",
        noun: "array/tuple/struct value",
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
