//! Tree-sitter structural text objects (`m i f`, `m a c`, …) and navigation
//! (`goto-next-<kind>`, `goto-prev-<kind>`) — the `SelectionBody::Structural`
//! interpreter and its two supporting steps: bringing a buffer's syntax tree
//! up to date before a query, and collecting the `ObjectSpans` a
//! `StructuralBody` probes.

use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_ops::MotionMode;
use hume_ops::motion::apply_object_motion;
use hume_ops::text_object::{
    apply_text_object_by_mode, around_argument, around_from_inner, inner_argument,
};
use hume_treesitter::syntax::Syntax;
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan, ObjectSpans};

use super::super::EditorState;
use super::super::buffer::Buffer;
use super::super::registry::StructuralBody;
use super::super::syntax::report_chain_break;

/// Bring `bid`'s committed tree up to date with its current text,
/// synchronously, before a structural command reads it.
///
/// A structural command runs after `Editor::settle` has already ticked the
/// frame's async reparse for the *previous* edit, and a macro or dot-repeat
/// batch replays several edits with no settle in between — either way the
/// committed tree can be a generation behind by the time this query needs
/// it. `Syntax::ensure_current` closes that window; this wrapper resolves
/// the borrows it needs (an `Arc` grammar snapshot, an O(1) rope clone) and
/// routes any `ChainBreak` through the same trace report
/// `reparse_stale_buffers` uses.
///
/// No-op when the buffer has no syntax attached at all (no grammar, or over
/// `syntax-highlight-max-bytes`) — the caller's `object_spans` then collects
/// nothing, which is the same "no grammar" no-op every structural command
/// already has. Also a no-op — rather than a blocking parse — in three cases
/// a fresh reparse here cannot help:
///
/// - **No committed tree yet.** Before the worker's first parse lands,
///   `build_request` has no `old_tree` to diff against, so this would run a
///   full parse of the whole buffer (up to `syntax-highlight-max-bytes`) on
///   the UI thread while the worker parses the identical bytes in the
///   background. `object_spans` already returns `ObjectSpans::default()`
///   when `layers` is `None`, so the command reads as the same "no grammar"
///   no-op until the next frame installs the worker's result.
/// - **Over the byte cap.** `Editor::reparse_stale_buffers` detaches syntax
///   from an over-cap buffer, but only once a frame — a paste that grows a
///   buffer past the cap is not yet detached if a structural keypress lands
///   in the same input batch. Checked here too rather than parsing the whole
///   buffer once before the next frame catches up.
/// - **No layer defines a textobjects query.** A grammar with no
///   `textobjects.scm` (most of them — PLUM's fetch is best-effort) can
///   never make `object_spans` return anything either way, so reparsing to
///   answer it is wasted work, worst on a `.`-repeat or macro batch that
///   pays it once per step. Misses one case: an edit that introduces a
///   *new* injected layer carrying a textobjects query is missed for this
///   one keypress — the next command call sees it.
pub(in crate::editor) fn ensure_syntax_current(state: &mut EditorState, bid: BufferId) {
    let buf = state.buffers.get(bid);
    let text_gen = buf.text_gen;
    let Some(syn) = buf.syntax.as_ref() else {
        return;
    };
    if syn.parsed_gen() == Some(text_gen) {
        return;
    }
    let Some(layers) = syn.layers() else {
        return;
    };
    if buf.text().len_bytes() > state.settings.syntax_highlight_max_bytes {
        return;
    }
    let has_textobjects = syn.bundle().textobjects.is_some()
        || layers
            .layers
            .iter()
            .any(|layer| layer.bundle.textobjects.is_some());
    if !has_textobjects {
        return;
    }

    let text = state.buffers.get(bid).text().clone();
    let langs = state.config.languages.grammar_snapshot();
    let syn = state
        .buffers
        .get_mut(bid)
        .syntax
        .as_mut()
        .expect("syntax is_some checked above");
    if let Some(brk) = syn.ensure_current(bid, text_gen, &text, &langs) {
        report_chain_break(state, bid, &brk);
    }
}

/// The `ObjectSpans` a `StructuralBody` probes against, for the object kind
/// (and, for `Goto`, the navigation span priority) that body needs. Shared
/// borrows only — the pipeline arm calling this still holds `&state.buffers`
/// when it does. `ObjectSpans::default()` (empty) when the buffer has no
/// syntax, no layers yet, or no layer defines the capture: `enclosing`/
/// `adjacent` then answer "nothing here" through the same code path every
/// other probe uses, making `Select`/`Goto` silent no-ops with no early
/// return.
pub(super) fn object_spans(buf: &Buffer, body: StructuralBody) -> ObjectSpans {
    let Some(layers) = buf.syntax.as_ref().and_then(Syntax::layers) else {
        return ObjectSpans::default();
    };
    let text = buf.text();
    match body {
        StructuralBody::Select { kind, span } => ObjectSpans::collect(layers, text, kind, span),
        StructuralBody::Goto { kind, .. } => {
            ObjectSpans::collect_for_navigation(layers, text, kind)
        }
        // `parameter.inside` (not `.around`, which Helix hulls with the
        // trailing comma `m i a`/`m a a` deliberately reject) — the same
        // span the lexical `inner_argument` fallback below produces.
        StructuralBody::Argument { .. } => {
            ObjectSpans::collect(layers, text, ObjectKind::Parameter, ObjectSpan::Inside)
        }
    }
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
    /// scan's `2`; array/tuple/struct members are `entry` objects (`m i e`).
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
            StructuralBody::Goto { dir, .. } => apply_object_motion(
                text,
                sels,
                mode,
                count,
                dir == Direction::Backward,
                |_, p| spans.adjacent(p, dir),
            ),
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
