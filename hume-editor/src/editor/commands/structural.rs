//! Tree-sitter structural text objects (`m i f`, `m a c`, …) and navigation
//! (`goto-next-<kind>`, `goto-prev-<kind>`): the `SelectionBody::Structural`
//! interpreter, the `ObjectSpans` collection a `StructuralBody` probes, and
//! the lexical scans a kind falls back to without one.
//!
//! Tree *freshness* before a query is `syntax::ensure_syntax_current`, next to
//! the per-frame reparse path it mirrors rather than here.

use std::sync::Arc;

use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_ops::MotionMode;
use hume_ops::motion::{adjacent_tag, apply_object_motion};
use hume_ops::text_object::{
    apply_text_object_by_mode, around_argument, around_from_inner, around_tag, inner_argument,
    inner_tag,
};
use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan, ObjectSpans, SpanSelector};

use super::super::buffer::Buffer;
use super::super::registry::StructuralBody;

/// The spans this body probes.
///
/// `Argument` resolves to `parameter.inside`, not `.around`, which Helix
/// hulls with the trailing comma `m i a`/`m a a` reject, and the
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

type FindObject = fn(&BufferText, ClusterStart) -> Option<ClusterRange>;

/// A kind's lexical form, defined once for both of its uses: `Select` falls
/// back to `inner`/`around` where no captured object encloses the cursor,
/// and `Goto` walks `adjacent` in a buffer whose grammar does not define the
/// kind at all. Navigation never mixes the two per step, or a forward and a
/// backward walk would visit different objects.
#[derive(Clone, Copy)]
struct LexicalObject {
    inner: FindObject,
    around: FindObject,
    adjacent: fn(&BufferText, ClusterStart, bool) -> Option<ClusterRange>,
}

/// `None` for a kind found only through a grammar. `Parameter` is one of
/// those here: its lexical scan belongs to `Argument`'s own rule, whose
/// "around" is derived from the inside span.
fn lexical_object(kind: ObjectKind) -> Option<LexicalObject> {
    match kind {
        ObjectKind::XmlElement => Some(LexicalObject {
            inner: inner_tag,
            around: around_tag,
            adjacent: adjacent_tag,
        }),
        ObjectKind::Function
        | ObjectKind::Class
        | ObjectKind::Parameter
        | ObjectKind::Comment
        | ObjectKind::Test
        | ObjectKind::Entry => None,
    }
}

/// The `ObjectSpans` a `StructuralBody` probes against. Shared borrows only:
/// the pipeline arm calling this still holds `&state.buffers` when it does,
/// and the memo behind `for_selector` is why that stays true.
///
/// An empty set (via `Arc<ObjectSpans>::default`) when the buffer has no
/// syntax, no layers yet, or layers of an older text (a buffer over the size
/// cap is never reparsed on demand): `enclosing`/`adjacent` then answer
/// "nothing here" through the same code path every other probe uses, making
/// `Select`/`Goto` no-ops with no early return.
pub(super) fn object_spans(buf: &Buffer, body: StructuralBody) -> Arc<ObjectSpans> {
    let Some(layers) = buf.current_layers() else {
        return Arc::default();
    };
    ObjectSpans::for_selector(layers, buf.text(), selector_for(body))
}

impl StructuralBody {
    /// Interpret this body against `spans`: the one place `Select`, `Goto`,
    /// and `Argument` become an actual selection transform, in place of a
    /// near-identical thin command function per structural command name.
    ///
    /// The `Argument` fallback to the lexical scan is per-probe, not per
    /// buffer: a comma list the query doesn't cover (a top-level array
    /// literal), a region under a syntax error, and a scratch buffer with no
    /// grammar all use the lexical scan. Where a
    /// tree span exists it wins outright: `m i a` on `2` in `foo([1, 2,
    /// 3])` selects the whole array (the call's argument), not the lexical
    /// scan's `2`; array/tuple/struct members are `entry`-kind objects,
    /// exposed as the `value` text object (`m i v`).
    pub(in crate::editor) fn apply(
        self,
        state: EditState,
        count: usize,
        mode: MotionMode,
        spans: &ObjectSpans,
    ) -> EditState {
        let enclosing = |_: &BufferText, p: ClusterStart| spans.enclosing(p);
        match self {
            StructuralBody::Select { kind, span } => {
                let lexical = lexical_object(kind).and_then(|l| match span {
                    ObjectSpan::Inside => Some(l.inner),
                    ObjectSpan::Around => Some(l.around),
                    ObjectSpan::Movement => None,
                });
                apply_text_object_by_mode(state, mode, |t, p| {
                    enclosing(t, p).or_else(|| lexical.and_then(|find| find(t, p)))
                })
            }
            StructuralBody::Goto { kind, dir } => {
                let backward = dir == Direction::Backward;
                let lexical = lexical_object(kind).filter(|_| !spans.grammar_defines());
                let text = state.text().clone();
                apply_object_motion(state, mode, count, backward, |p| match lexical {
                    Some(l) => (l.adjacent)(&text, p, backward),
                    None => spans.adjacent(p, dir),
                })
            }
            StructuralBody::Argument { around: false } => {
                apply_text_object_by_mode(state, mode, |t, p| {
                    enclosing(t, p).or_else(|| inner_argument(t, p))
                })
            }
            StructuralBody::Argument { around: true } => {
                apply_text_object_by_mode(state, mode, |t, p| {
                    enclosing(t, p)
                        .map(|s| around_from_inner(t, s))
                        .or_else(|| around_argument(t, p))
                })
            }
        }
    }
}
