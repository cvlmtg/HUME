//! Structural object navigation: the Move/Extend/count policy behind
//! `goto-next-<kind>` / `goto-prev-<kind>`, parameterized over a `finder`
//! rather than a tree: this crate cannot depend on `hume-treesitter`, so
//! `hume-editor` supplies `finder` as a closure over
//! `hume_treesitter::textobjects::ObjectSpans::adjacent` for the tree-sitter
//! kinds. The paragraph motions (`super::paragraph`) are a second, in-crate
//! caller whose `finder` is a lexical blank-line scan instead. `apply_object_motion`
//! only cares that `finder` returns the object's clusters and honors the
//! strict-progress contract described below, not how the object was found.

use super::MotionMode;
use hume_editing::selection::{Facing, Selection};
use hume_editing::state::EditState;
use hume_rope::cluster::{ClusterRange, ClusterStart};

/// Apply structural navigation to every selection, repeated `count` times.
/// `finder` maps an origin to a whole object; `backward` picks the search
/// direction and which edge becomes the head.
///
/// **Move**: origin is the selection's last cluster forward, its first
/// backward, so a repeated press skips objects nested in the one just
/// selected. The result faces backward in both directions (head at the
/// object's start), so the viewport lands on the object's signature.
///
/// **Extend**: origin is the head, since searching from a Move result's
/// reversed anchor would skip every object up to the head. The result covers
/// the current selection and the object together, not an edge replacement:
/// the object may be nested inside the current selection, and a replacement
/// would drop everything past its end.
///
/// `finder` must make strict progress (start past the origin forward, before
/// it backward), so no fixed-point check is needed. `None` stops that
/// selection's loop and keeps its last result. Converging cursors merge.
pub fn apply_object_motion(
    state: EditState,
    mode: MotionMode,
    count: usize,
    backward: bool,
    finder: impl Fn(ClusterStart) -> Option<ClusterRange>,
) -> EditState {
    state.map(|sel| {
        let mut current = sel.selection();
        let mut covered = sel.covered();
        for _ in 0..count {
            let origin = match mode {
                MotionMode::Move if backward => covered.start(),
                MotionMode::Move => covered.last(),
                MotionMode::Extend => current.head(),
            };
            let Some(object) = finder(origin) else {
                break;
            };
            (current, covered) = match mode {
                MotionMode::Move => (Selection::covering(object, Facing::Backward), object),
                MotionMode::Extend => {
                    let union = covered.hull(object);
                    (
                        Selection::covering(union, Facing::from_forward(!backward)),
                        union,
                    )
                }
            };
        }
        current
    })
}
