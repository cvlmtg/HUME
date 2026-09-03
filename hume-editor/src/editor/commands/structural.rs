//! Tree-sitter structural text objects (`m i f`, `m a c`, …) and navigation
//! (`goto-next-<kind>`, `goto-prev-<kind>`) — the `SelectionBody::Structural`
//! interpreter and the `ObjectSpans` collection a `StructuralBody` probes.
//!
//! Tree *freshness* before a query is `syntax::ensure_syntax_current`, next to
//! the per-frame reparse path it mirrors rather than here.

use std::sync::Arc;

use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_ops::MotionMode;
use hume_ops::motion::apply_object_motion;
use hume_ops::text_object::{
    apply_text_object_by_mode, around_argument, around_from_inner, inner_argument,
};
use hume_treesitter::syntax::Syntax;
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan, ObjectSpans, SpanSelector};

use super::super::buffer::Buffer;
use super::super::registry::StructuralBody;

/// The spans this body probes.
///
/// `Argument` resolves to `parameter.inside` — not `.around`, which Helix
/// hulls with the trailing comma `m i a`/`m a a` deliberately reject, and the
/// same span the lexical `inner_argument` fallback produces.
fn selector_for(body: StructuralBody) -> SpanSelector {
    match body {
        StructuralBody::Select { kind, span } => SpanSelector::Exact(kind, span),
        StructuralBody::Goto { kind, .. } => SpanSelector::Navigation(kind),
        StructuralBody::Argument { .. } => {
            SpanSelector::Exact(ObjectKind::Parameter, ObjectSpan::Inside)
        }
    }
}

/// The `ObjectSpans` a `StructuralBody` probes against. Shared borrows only —
/// the pipeline arm calling this still holds `&state.buffers` when it does,
/// and the memo behind `for_selector` is why that stays true.
///
/// An empty set (via `Arc<ObjectSpans>::default`) when the buffer has no
/// syntax or no layers yet: `enclosing`/`adjacent` then answer "nothing here"
/// through the same code path every other probe uses, making `Select`/`Goto`
/// silent no-ops with no early return.
pub(super) fn object_spans(buf: &Buffer, body: StructuralBody) -> Arc<ObjectSpans> {
    let Some(layers) = buf.syntax.as_ref().and_then(Syntax::layers) else {
        return Arc::default();
    };
    ObjectSpans::for_selector(layers, buf.text(), selector_for(body))
}

impl StructuralBody {
    /// Interpret this body against `spans`: the one place `Select`, `Goto`,
    /// and `Argument` become an actual `SelectionSet` transform, replacing
    /// the 22 near-identical thin command functions a per-command
    /// implementation would otherwise need.
    ///
    /// The `Argument` fallback to the lexical scan is per-probe, not per
    /// buffer: a comma list the query doesn't cover (a top-level array
    /// literal), a region under a syntax error, and a scratch buffer with no
    /// grammar all behave exactly as they did before this feature. Where a
    /// tree span exists it wins outright — `m i a` on `2` in `foo([1, 2,
    /// 3])` selects the whole array (the call's argument), not the lexical
    /// scan's `2`; array/tuple/struct members are `entry`-kind objects,
    /// exposed as the `value` text object (`m i v`).
    pub(in crate::editor) fn apply(
        self,
        text: &BufferText,
        sels: SelectionSet,
        count: usize,
        mode: MotionMode,
        spans: &ObjectSpans,
    ) -> SelectionSet {
        match self {
            StructuralBody::Select { .. } => {
                apply_text_object_by_mode(text, sels, mode, |_, p| spans.enclosing(p))
            }
            StructuralBody::Goto { dir, .. } => {
                apply_object_motion(text, sels, mode, count, dir == Direction::Backward, |p| {
                    spans.adjacent(p, dir)
                })
            }
            StructuralBody::Argument { around: false } => {
                apply_text_object_by_mode(text, sels, mode, |t, p| {
                    spans.enclosing(p).or_else(|| inner_argument(t, p))
                })
            }
            StructuralBody::Argument { around: true } => {
                apply_text_object_by_mode(text, sels, mode, |t, p| {
                    spans
                        .enclosing(p)
                        .map(|s| around_from_inner(t, s))
                        .or_else(|| around_argument(t, p))
                })
            }
        }
    }
}
