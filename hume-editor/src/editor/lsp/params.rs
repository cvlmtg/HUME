//! Ready-made request params: a document URI with opaque positions, which
//! each server receives in its own encoding when the request is sent. No
//! server is consulted, so a buffer with a path always has params.

use hume_editing::text::TextVersion;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_rope::cluster::{ClusterBound, ClusterRange};
use hume_rope::offset::CharOffset;
use hume_scripting::host::{PositionParams, RangeParams, RangesParams};
use hume_scripting::{DocPos, DocRange};

use crate::editor::EditorState;
use crate::editor::commands::{CommandPane, pane_view};

fn doc_range(bid: BufferId, version: TextVersion, range: ClusterRange) -> DocRange {
    let chars = range.chars();
    DocRange {
        buffer: bid,
        version,
        start: chars.start,
        end: chars.end,
    }
}

/// The primary cursor head in `t`'s own pane. `None` only when `t`'s buffer
/// has no path.
pub(in crate::editor) fn position_params(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> Option<PositionParams> {
    let head = pane_view(state, view, t).primary().head();
    offset_params(state, t.bid(view), head.offset())
}

/// [`position_params`] for `offset` of `bid`'s live text instead of a
/// pane's cursor. `None` when `bid` is closed or has no path.
pub(in crate::editor) fn offset_params(
    state: &EditorState,
    bid: BufferId,
    offset: CharOffset,
) -> Option<PositionParams> {
    Some(PositionParams {
        uri: state.lsp_doc_uri(bid)?.as_str().to_string(),
        pos: DocPos {
            buffer: bid,
            version: state.buffers.try_get(bid)?.text().version(),
            offset,
        },
    })
}

/// The range the primary selection covers in `t`'s own pane: the shape
/// `:lsp-code-actions` needs, since its diagnostics context is
/// primary-scoped too.
pub(in crate::editor) fn primary_range_params(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> Option<RangeParams> {
    let bid = t.bid(view);
    let covered = pane_view(state, view, t).primary().covered();
    Some(RangeParams {
        uri: state.lsp_doc_uri(bid)?.as_str().to_string(),
        range: doc_range(bid, state.buffers.get(bid).text().version(), covered),
    })
}

/// One range per *linewise* selection in `t`'s pane, run-length-coalesced:
/// a run of selections that touch end-to-end (the next starts where the
/// previous ends) collapses into one range, since an LSP range is
/// contiguous. A non-linewise selection is skipped: the caller decides what
/// an all-linewise, all-partial, or mixed selection set means. An ambiguous
/// selection (see `SelectionView::linewise_classification`) is skipped the
/// same way, including from the touch check, so a stray cursor can't bridge
/// two linewise neighbors into one range that reformats the blank line
/// between them too. `None` only when `t`'s buffer has no path.
pub(in crate::editor) fn linewise_ranges_params(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> Option<RangesParams> {
    let bid = t.bid(view);
    let uri = state.lsp_doc_uri(bid)?.as_str().to_string();
    let text = state.buffers.get(bid).text();
    let selections = t.state(&state.panes.state, view).view(text);
    let linewise: Vec<ClusterRange> = selections
        .iter()
        .filter(|sel| sel.linewise_classification() == Some(true))
        .map(|sel| sel.covered())
        .collect();
    let ranges = linewise
        .chunk_by(|a, b| ClusterBound::from(b.start()) == a.end())
        .map(|run| doc_range(bid, text.version(), run[0].hull(run[run.len() - 1])))
        .collect();
    Some(RangesParams { uri, ranges })
}
