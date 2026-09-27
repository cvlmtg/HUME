//! Structural object navigation — the Move/Extend/count policy behind
//! `goto-next-<kind>` / `goto-prev-<kind>`, parameterized over a `finder`
//! rather than a tree: this crate cannot depend on `hume-treesitter`, so
//! `hume-editor` supplies `finder` as a closure over
//! `hume_treesitter::textobjects::ObjectSpans::adjacent` for the tree-sitter
//! kinds. The paragraph motions (`super::paragraph`) are a second, in-crate
//! caller whose `finder` is a lexical blank-line scan instead — `apply_object_motion`
//! only cares that `finder` returns `Option<InclusiveRange<CharOffset>>` and
//! honors the strict-progress contract described below, not how the span was
//! found.

use super::MotionMode;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, InclusiveRange};

/// Apply structural navigation to every selection in the set, repeated
/// `count` times. `finder` maps an origin to a whole object span; `backward`
/// picks the search direction and which span edge becomes the head.
///
/// **Move**: origin is `current.end()` forward, `current.start()` backward,
/// so a repeated press skips objects nested in the one just selected. The
/// result has anchor at the object's end and head at its start in both
/// directions, so the viewport lands on the object's signature.
///
/// **Extend**: origin is `current.head()`, since searching from a Move
/// result's reversed anchor would skip every object up to the head. The
/// result is `current.union_span(span, !backward)`, not an edge replacement:
/// the found span may be nested inside the current selection, and a
/// replacement would drop everything past its end.
///
/// `finder` must make strict progress (`start > pos` forward, `start < pos`
/// backward), so no fixed-point check is needed. `None` stops that
/// selection's loop and keeps its last result. Converging cursors merge.
pub fn apply_object_motion(
    text: &BufferText,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    backward: bool,
    finder: impl Fn(CharOffset) -> Option<InclusiveRange<CharOffset>>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let mut current = sel;
        for _ in 0..count {
            let origin = match mode {
                MotionMode::Move if backward => current.start(),
                MotionMode::Move => current.end(),
                MotionMode::Extend => current.head(),
            };
            let Some(span) = finder(origin) else {
                break;
            };
            current = match mode {
                MotionMode::Move => Selection::new(span.end, span.start),
                MotionMode::Extend => current.union_span(span, !backward),
            };
        }
        current
    });
    result.debug_assert_valid(text);
    result
}
