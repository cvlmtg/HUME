//! Ready-made request params: an opaque document with opaque positions,
//! which each server receives as its own URI and encoding when the request
//! is sent. No server is consulted, so every buffer has params.

use hume_core::BufferId;
use hume_engine::pipeline::EngineView;
use hume_rope::cluster::{ClusterBound, ClusterRange};
use hume_rope::offset::CharOffset;
use hume_scripting::host::{PositionParams, RangeParams, RangesParams};
use hume_scripting::{DocPos, DocRange, DocUri};

use crate::editor::EditorState;
use crate::editor::commands::{CommandPane, pane_view};

/// The primary cursor head in `t`'s own pane.
pub(in crate::editor) fn position_params(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> PositionParams {
    let bid = t.bid(view);
    let head = pane_view(state, view, t).primary().head();
    PositionParams {
        doc: DocUri::new(bid),
        pos: DocPos::new(bid, state.buffers.get(bid).text(), head.offset()),
    }
}

/// [`position_params`] for `offset` of `bid`'s live text instead of a
/// pane's cursor. `None` when `bid` is closed.
pub(in crate::editor) fn offset_params(
    state: &EditorState,
    bid: BufferId,
    offset: CharOffset,
) -> Option<PositionParams> {
    Some(PositionParams {
        doc: DocUri::new(bid),
        pos: DocPos::new(bid, state.buffers.try_get(bid)?.text(), offset),
    })
}

/// The range the primary selection covers in `t`'s own pane: the shape
/// `lsp/code-actions` needs, since its diagnostics context is
/// primary-scoped too.
pub(in crate::editor) fn primary_range_params(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> RangeParams {
    let bid = t.bid(view);
    let covered = pane_view(state, view, t).primary().covered();
    RangeParams {
        doc: DocUri::new(bid),
        range: DocRange::new(bid, state.buffers.get(bid).text(), covered),
    }
}

/// One range per *linewise* selection in `t`'s pane, run-length-coalesced:
/// a run of selections that touch end-to-end (the next starts where the
/// previous ends) collapses into one range, since an LSP range is
/// contiguous. A non-linewise selection is skipped: the caller decides what
/// an all-linewise, all-partial, or mixed selection set means. An ambiguous
/// selection (see `SelectionView::linewise_classification`) is skipped the
/// same way, including from the touch check, so a stray cursor can't bridge
/// two linewise neighbors into one range that reformats the blank line
/// between them too.
pub(in crate::editor) fn linewise_ranges_params(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> RangesParams {
    let bid = t.bid(view);
    let text = state.buffers.get(bid).text();
    let selections = t.state(&state.panes.entries, view).view(text);
    let linewise: Vec<ClusterRange> = selections
        .iter()
        .filter(|sel| sel.linewise_classification() == Some(true))
        .map(|sel| sel.covered())
        .collect();
    let ranges = linewise
        .chunk_by(|a, b| ClusterBound::from(b.start()) == a.end())
        .map(|run| DocRange::new(bid, text, run[0].hull(run[run.len() - 1])))
        .collect();
    RangesParams {
        doc: DocUri::new(bid),
        ranges,
    }
}
