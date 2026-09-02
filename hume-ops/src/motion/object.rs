//! Structural object navigation — the Move/Extend/count policy behind
//! `goto-next-<kind>` / `goto-prev-<kind>`, parameterized over a `finder`
//! rather than a tree: this crate cannot depend on `hume-treesitter`, so the
//! caller (`hume-editor`) supplies `finder` as
//! `hume_treesitter::textobjects::ObjectSpans::adjacent`, whose
//! `Option<(usize, usize)>` contract this module's `finder` parameter
//! matches exactly.

use super::MotionMode;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;

/// Apply structural navigation to every selection in the set, repeated
/// `count` times.
///
/// Unlike [`super::apply_motion`], which maps a search origin to a new head
/// position, `finder` maps an origin to a whole object span. `backward`
/// selects which edge of the current selection each step searches from and
/// which edge of the found span becomes the new head in `Extend` mode — the
/// `apply_word_select` convention, taken rather than importing
/// `hume_treesitter::textobjects::Direction` at this crate's boundary.
///
/// **Move**: origin is `current.end()` going forward, `current.start()`
/// going backward. Searching from the far edge of the selection just made
/// (rather than its near edge) is what causes a second forward press to
/// skip past an object nested inside the one just selected, instead of
/// re-finding it. The result is `Selection::new(end, start)`: anchor at the
/// object's **end**, head at its **start** in both directions, so the
/// viewport lands on the object's signature and a following `w` walks into
/// its body.
///
/// **Extend**: origin is always `current.head()`; the anchor stays pinned to
/// the *original* selection's anchor across every step, as
/// [`super::apply_motion`] does, while each step's head becomes the found
/// span's start (backward) or end (forward) — growing the selection to
/// cover the object.
///
/// `finder` returning `None` stops the loop early for that selection and
/// keeps its last result, so a `count` past the last object leaves the
/// selection on that last object rather than clearing it. No fixed-point
/// check is needed here (unlike `apply_motion`): `adjacent` requires strict
/// progress (`start > pos` forward, `start < pos` backward), so every
/// successful step actually moves and a stalled search returns `None`
/// rather than repeating a position.
///
/// Uses `map` (which always merges) so that cursors converging on the same
/// object are merged, exactly as `apply_motion` and `apply_text_object` do.
pub fn apply_object_motion(
    text: &BufferText,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    backward: bool,
    finder: impl Fn(&BufferText, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let mut current = sel;
        for _ in 0..count {
            let origin = if backward {
                current.start()
            } else {
                current.end()
            };
            let Some((start, end)) = finder(text, origin) else {
                break;
            };
            current = match mode {
                MotionMode::Move => Selection::new(end, start),
                MotionMode::Extend => {
                    Selection::new(sel.anchor(), if backward { start } else { end })
                }
            };
        }
        current
    });
    result.debug_assert_valid(text);
    result
}
