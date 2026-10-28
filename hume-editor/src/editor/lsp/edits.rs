//! Edit + navigation primitives: `apply-text-edits!`,
//! `apply-workspace-edit!`, `goto-location!`, and the char-range commit
//! `edit-text!` shares with them.
//!
//! Everything here is state/view-only (no `&mut Editor` needed), so it is
//! callable directly from `EditorHostImpl`, the same discipline as the
//! decoration setters. `apply_edit_request_response` answers a
//! server-initiated request. It lives here rather than in `drain.rs` because
//! it shares the edit-application path with `apply-workspace-edit!` and
//! belongs next to it.

use hume_core::{BufferId, NonEmpty, OnUnsaved, PaneHandle, Severity};
use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::apply_keeping_final_break;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{EngineView, PanePool};
use hume_lsp::codec::ResponseError;
use hume_platform::path::AbsolutePath;
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_rope::position_encoding::{PositionEncoding, WirePos};

use super::introspect;
use crate::editor::EditorState;
use crate::editor::buffer::Buffer;
use crate::editor::commands::{FocusedPane, shown_through};
use crate::editor::doc_ops;

/// Shared precondition check for every edit entry point in this module: the
/// buffer exists, is writable, and hasn't moved since the caller computed its
/// positions against it. A caller reading positions out of its edits checks
/// this before reading them, so a stale edit is reported as stale rather than
/// as a position the current text lacks.
pub(in crate::editor) fn checked_buffer(
    state: &EditorState,
    bid: BufferId,
    expect_gen: Option<u64>,
) -> Result<&Buffer, String> {
    let Some(buf) = state.buffers.try_get(bid) else {
        return Err("no such buffer".to_string());
    };
    buf.check_text_change()?;
    match expect_gen {
        Some(expected) if buf.text().generation() != expected => {
            Err("buffer changed since these edits were computed".to_string())
        }
        _ => Ok(buf),
    }
}

/// Converts `edits` (wire positions, any order) into one composite
/// `ChangeSet`: positions are resolved to char offsets, stable-sorted
/// ascending by start then end (so an insertion goes before a range starting
/// at the same offset, and equal ranges keep `edits`' own array order, per
/// spec), checked for overlap (adjacent edits are fine; a shared boundary is
/// not), then walked forward once to build the `ChangeSet`. Read-only, no
/// mutation: `apply_workspace_edit`'s "validate all, then apply all" needs
/// to build every file's changeset before committing any of them.
///
/// `encoding` is the caller's to resolve, not this function's: a workspace
/// edit's every file shares the one server that produced the whole edit
/// (resolved once via `server_encoding`), while `apply-text-edits!` reads
/// each edit handle's own tagged producing-server encoding
/// (`JsonHandle::position_encoding`), never `bid`'s *currently* attached
/// server; see each caller for which.
fn build_edit_changeset(
    state: &EditorState,
    bid: BufferId,
    edits: &NonEmpty<(ExclusiveRange<WirePos>, &str)>,
    expect_gen: Option<u64>,
    encoding: PositionEncoding,
) -> Result<ChangeSet, String> {
    let buf = checked_buffer(state, bid, expect_gen)?;
    let rope = buf.text().rope();
    let char_edits = edits
        .iter()
        .map(|&(range, new_text)| Ok((edit_range_to_chars(rope, range, encoding)?, new_text)))
        .collect::<Result<Vec<(ExclusiveRange<CharOffset>, &str)>, String>>()?;
    build_changeset_from_char_edits(buf.text(), char_edits)
}

/// `range`, a server's edit range against `rope`, as chars in `encoding`.
/// Refused when it ends on a line past the document end, which no clamp can
/// honestly stand in for, or ends before it starts.
pub(in crate::editor) fn edit_range_to_chars(
    rope: &ropey::Rope,
    range: ExclusiveRange<WirePos>,
    encoding: PositionEncoding,
) -> Result<ExclusiveRange<CharOffset>, String> {
    let chars =
        hume_rope::position_encoding::checked_wire_range_to_char_range(rope, range, encoding)
            .ok_or_else(|| past_document_end(range, rope))?;
    if chars.end < chars.start {
        return Err(reversed_range(chars));
    }
    Ok(chars)
}

/// Why an edit range that ends before it starts is refused.
fn reversed_range(range: ExclusiveRange<CharOffset>) -> String {
    format!(
        "text edit has a reversed range (end {:?} before start {:?})",
        range.end, range.start
    )
}

/// Why a text edit whose range ends on a line past `rope`'s document end is
/// refused.
fn past_document_end(range: ExclusiveRange<WirePos>, rope: &ropey::Rope) -> String {
    format!(
        "text edit range ends past the end of the document (line {}, which has {} lines)",
        range.start.line.max(range.end.line),
        hume_rope::lines::content_line_count(rope).get()
    )
}

/// Shared tail for [`build_edit_changeset`] (wire positions, converted to
/// char offsets above), the completion-resolve path (already char offsets,
/// mapped forward through the accept edit) and `edit-text!` (char offsets
/// from Steel, checked by the host): sort, overlap-check, then walk once to
/// build the `ChangeSet`. `edits` need not arrive sorted; `new_text` borrows
/// from the caller's own edit list. A server's `new_text` is under no
/// obligation to use LF, but needs no handling here: the changeset builder
/// normalizes every insertion.
///
/// A server works on a file that may not end with a newline, while the
/// buffer always does: an edit that drops the final `\n` gets it back under
/// [`apply_keeping_final_break`]'s rule.
fn build_changeset_from_char_edits(
    text: &BufferText,
    mut char_edits: Vec<(ExclusiveRange<CharOffset>, &str)>,
) -> Result<ChangeSet, String> {
    if let Some((range, _)) = char_edits.iter().find(|(r, _)| r.end < r.start) {
        return Err(reversed_range(*range));
    }
    // An insertion sorts before a range that starts where it does, and edits
    // with equal ranges keep their list order.
    char_edits.sort_by_key(|e| (e.0.start, e.0.end));
    for w in char_edits.windows(2) {
        if w[1].0.start < w[0].0.end {
            return Err("text edits overlap".to_string());
        }
    }

    let end = text.end();
    let inserts_at_end = char_edits
        .iter()
        .any(|(range, new_text)| range.start == end && !new_text.is_empty());
    let (_, changes, ()) = apply_keeping_final_break(text, |ceiling| {
        let mut b = ChangeSetBuilder::new(end);
        for (range, new_text) in &char_edits {
            let stop = match ceiling {
                Some(ceiling) if !inserts_at_end => range.end.min(ceiling),
                _ => range.end,
            };
            b.retain_to(range.start);
            if stop > range.start {
                b.delete_to(stop);
            }
            b.insert(new_text);
        }
        if ceiling.is_some() && inserts_at_end {
            b.insert("\n");
        }
        (b.finish(), ())
    });
    Ok(changes)
}

/// Applies a pre-built `ChangeSet` to `target`'s buffer as one undo step, through
/// the same chokepoint every native edit command uses
/// (`doc_ops::apply_changes`: selection propagation, syntax reparse,
/// queued LSP `didChange`, all for free). Returns the applied `ChangeSet` so
/// a caller that needs to map further positions through this exact edit
/// (completion's resolve path, mapping a *pre*-accept response's positions
/// forward) can do so without rebuilding it.
fn commit_changeset(
    state: &mut EditorState,
    target: PaneHandle,
    cs: ChangeSet,
) -> Result<ChangeSet, String> {
    let (buffers, mut stores) = state.edit_parts();
    doc_ops::apply_changes(buffers, &mut stores, target, cs.clone()).map_err(|e| e.to_string())?;
    Ok(cs)
}

/// `(apply-text-edits! pane edits #:expect-generation gen)`. `encoding` is
/// the host layer's own resolve: every entry in `edits` decoded from its
/// own tagged `JsonHandle`, checked there to all agree. The edit goes
/// through `requester`'s pane only while that pane shows its buffer.
pub(in crate::editor) fn apply_text_edits(
    state: &mut EditorState,
    panes: &PanePool,
    requester: PaneHandle,
    edits: &NonEmpty<(ExclusiveRange<WirePos>, &str)>,
    expect_gen: Option<u64>,
    encoding: PositionEncoding,
) -> Result<(), String> {
    let bid = requester.buffer();
    let cs = build_edit_changeset(state, bid, edits, expect_gen, encoding)?;
    commit_changeset(state, shown_through(panes, requester.pane(), bid), cs)?;
    Ok(())
}

/// Decodes `edits` (wire positions, computed by the server against the
/// document as it stood at `rope_at`) into char-offset `(start, end, text)`
/// triples valid against the document `cs_forward` transforms `rope_at`
/// into: exact position tracking through every edit `cs_forward` composes,
/// unlike a scalar-delta approximation. Pure: no buffer access, so it can run
/// before deciding whether the caller's own edit (if any) would overlap the
/// result.
///
/// Two callers, two documents-since-`rope_at`: LSP completion accept, where
/// `rope_at` is the request-time snapshot and `cs_forward` is every
/// keystroke observed since; and a `completionItem/resolve` response, where
/// `rope_at` is the pre-accept snapshot and `cs_forward` is the accept
/// edit's own changeset.
///
/// Returns an empty `Vec` when `edits` is empty: an empty edit list changes
/// nothing, as it does for `apply-text-edits!`.
pub(in crate::editor) fn build_edits_from_earlier_document<'a>(
    rope_at: &ropey::Rope,
    cs_forward: &ChangeSet,
    encoding: PositionEncoding,
    edits: &'a [lsp_types::TextEdit],
) -> Result<Vec<(ExclusiveRange<CharOffset>, &'a str)>, String> {
    if edits.is_empty() {
        return Ok(Vec::new());
    }
    let mut ranges = edits
        .iter()
        .map(|e| {
            edit_range_to_chars(
                rope_at,
                hume_lsp::position::from_lsp_range(&e.range),
                encoding,
            )
        })
        .collect::<Result<Vec<ExclusiveRange<CharOffset>>, String>>()?;
    // `map_ranges` requires sorted-by-start input; sort the (range, text)
    // pairing together so the mapped range still lines up with its own text
    // afterward.
    let mut indexed: Vec<usize> = (0..ranges.len()).collect();
    indexed.sort_by_key(|&i| ranges[i].start);
    ranges.sort_by_key(|r| r.start);
    cs_forward.map_ranges(&mut ranges);

    Ok(indexed
        .into_iter()
        .zip(ranges)
        .map(|(orig_i, range)| (range, edits[orig_i].new_text.as_str()))
        .collect())
}

/// Commits pre-computed char-offset edits (from
/// [`build_edits_from_earlier_document`] or any other char-space source)
/// against `requester`'s buffer's *current* text as one `ChangeSet`, through
/// `requester`'s pane while that pane shows the buffer. A caller whose edits
/// must match a generation checks it with [`checked_buffer`] before
/// computing them. Validates overlap/reversed-range
/// across the batch itself (via [`build_changeset_from_char_edits`])
/// immediately before mutating.
/// `Ok(None)` for an empty batch (nothing to commit); `Ok(Some(cs))`
/// otherwise, so a caller composing this into a larger changeset doesn't need
/// its own empty-batch branch.
pub(in crate::editor) fn commit_char_edits(
    state: &mut EditorState,
    panes: &PanePool,
    requester: PaneHandle,
    char_edits: Vec<(ExclusiveRange<CharOffset>, &str)>,
) -> Result<Option<ChangeSet>, String> {
    if char_edits.is_empty() {
        return Ok(None);
    }
    let bid = requester.buffer();
    let buf = checked_buffer(state, bid, None)?;
    let cs = build_changeset_from_char_edits(buf.text(), char_edits)?;
    let target = shown_through(panes, requester.pane(), bid);
    Ok(Some(commit_changeset(state, target, cs)?))
}

pub(in crate::editor) struct WorkspaceEditSummary {
    pub(in crate::editor) buffers_modified: usize,
}

/// Opens `path`, a URI's path made absolute against the editor cwd as
/// given, in a buffer, or finds the one already showing it. The flag is
/// whether this call opened it.
fn resolve_or_open(
    state: &mut EditorState,
    view: &mut EngineView,
    path: &std::path::Path,
) -> Result<(BufferId, bool), String> {
    // A server-driven rename or workspace edit may target a file that
    // doesn't exist yet, openable here as `:e` opens a missing path
    // (see `Buffer::from_file_or_new`).
    AbsolutePath::from_path(path, &state.cwd)
        .and_then(|typed| {
            crate::editor::buffer::lifecycle::open_or_dedup_and_notify(view, state, typed)
        })
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// `(apply-workspace-edit! edit)`: validates and builds every file's
/// changeset first (opening unopened files as buffers along the way), and
/// only commits any of them once every file has passed: a bad edit in file
/// 3 of 5 must leave files 1 and 2 untouched. Reports how many buffers it
/// changed, and nothing for an edit with nothing to apply.
///
/// `encoding` is the caller's own resolve (`edit`'s tagged producing-server
/// encoding) and shared by every file in `we`: one server produced the
/// whole edit, so every position in it is counted in that server's own
/// negotiated encoding, regardless of which file (open or not, attached to
/// a server or not) each position lands in. A per-file lookup would
/// misdecode a file with no server of its own attached, or one attached to
/// a different server, and (for a file this call is opening for the first
/// time) has no attached server to look up at all yet
/// (detection only runs after this returns).
pub(in crate::editor) fn apply_workspace_edit(
    state: &mut EditorState,
    view: &mut EngineView,
    requester: PaneHandle,
    we: lsp_types::WorkspaceEdit,
    encoding: PositionEncoding,
    expect_gen: Option<u64>,
) -> Result<WorkspaceEditSummary, String> {
    let entries = hume_lsp::workspace_edit::file_edits(we)?;
    if entries.is_empty() {
        return Ok(WorkspaceEditSummary {
            buffers_modified: 0,
        });
    }
    // Checked against `requester`'s buffer, the one the request was made
    // from, not any file the edit touches: "has it changed since," the same
    // staleness `apply-text-edits!`'s `#:expect-generation` guards. Per-file
    // staleness (a `documentChanges` entry's own `version`) is checked
    // separately, per entry, below.
    if expect_gen.is_some() {
        checked_buffer(state, requester.buffer(), expect_gen)?;
    }
    let mut planned: Vec<(PaneHandle, ChangeSet)> = Vec::with_capacity(entries.len());
    for hume_lsp::workspace_edit::FileEdits {
        uri,
        edits,
        version,
    } in entries
    {
        let path = hume_lsp::uri::uri_to_path(&uri).map_err(|e| format!("bad uri: {e:?}"))?;
        let (bid, _) = resolve_or_open(state, view, &path)?;
        // Each file's changeset is built against `state`'s *current* text:
        // a second entry for the same file would build a changeset that's
        // valid against that same original text, but `commit_changeset`
        // applies entries one at a time against whatever the buffer holds
        // *after* the previous commit. Position-based ops don't necessarily
        // fail to apply against the wrong text (they can silently produce
        // corrupted content instead of erroring), so this must be rejected
        // before any commit, not left to a downstream length coincidence.
        let display = || {
            state
                .buffers
                .get(bid)
                .display_path()
                .map(str::to_owned)
                .unwrap_or_else(|| path.display().to_string())
        };
        // Checked here, ahead of every file's own changeset build, not left
        // to `commit_changeset`'s own check at the bottom of this function:
        // a conflict discovered only at commit time would leave every
        // earlier file in this same batch already applied. See this
        // function's own doc for why that's the one thing a multi-file edit
        // must not do.
        let target = shown_through(&view.panes, requester.pane(), bid);
        crate::editor::edit_session::check_no_conflicting_session(state.open_session(), target)
            .map_err(|e| format!("{}: {e}", display()))?;
        if planned.iter().any(|(t, _)| t.buffer() == bid) {
            return Err(format!(
                "{}: workspace edit contains more than one entry for this file",
                display()
            ));
        }
        let expect_gen = version.map(|v| v as u64);
        let wire_edits = edits.map_ref(|e| {
            (
                hume_lsp::position::from_lsp_range(&e.range),
                e.new_text.as_str(),
            )
        });
        let cs = build_edit_changeset(state, bid, &wire_edits, expect_gen, encoding)
            .map_err(|e| format!("{}: {e}", display()))?;
        planned.push((target, cs));
    }
    let buffers_modified = planned.len();
    for (target, cs) in planned {
        commit_changeset(state, target, cs)?;
    }
    state.report(
        Severity::Info,
        format!("{buffers_modified} buffers modified — :wa writes all"),
    );
    Ok(WorkspaceEditSummary { buffers_modified })
}

/// `(goto-location! pane target)`'s two shapes that come from outside and
/// may be stale:
/// - a raw `Location`/`LocationLink` (wire positions, converted with
///   `encoding`, the host trait layer's own read of the handle's tagged
///   producing-server encoding, resolved before this ever needing a `bid`
///   or live `LspState` lookup);
/// - `(hash 'target path 'line 'char-col)`, already char-indexed.
///
/// Either names a place that no longer exists when its line is past the
/// last content line (a server's answer, a file changed on disk), and the
/// jump does not happen. The pane shape is [`goto_buffer_offset`]'s.
pub(in crate::editor) enum GotoTarget {
    Wire {
        uri: lsp_types::Uri,
        pos: WirePos,
        encoding: PositionEncoding,
    },
    Path {
        path: AbsolutePath,
        line: usize,
        char_col: hume_rope::column::CharCol,
    },
}

/// The cluster at `(line, char_col)` in `bid`: the column clamps and snaps
/// to the line's content, but a `line` past the last content line names no
/// place, so `None`.
fn char_indexed_to_cluster(
    state: &EditorState,
    bid: BufferId,
    line: usize,
    char_col: hume_rope::column::CharCol,
) -> Option<ClusterStart> {
    let text = state.buffers.get(bid).text();
    let line = hume_rope::line::ContentLine::checked(text.rope(), line)?;
    Some(
        text.columns()
            .place_char(hume_rope::line::RopeyLine::from(line), char_col),
    )
}

/// `landed` as a jump destination in `bid`. A buffer this call opened for a
/// destination that does not exist is closed again, so a stale target leaves
/// no trace.
fn settle_opened(
    state: &mut EditorState,
    view: &mut EngineView,
    bid: BufferId,
    opened: bool,
    landed: Option<ClusterStart>,
) -> Option<(BufferId, ClusterStart)> {
    if landed.is_none() && opened {
        crate::editor::buffer::lifecycle::close_buffer_and_notify(
            view,
            state,
            bid,
            OnUnsaved::Refuse,
        )
        .expect("a buffer this call just opened has no unsaved changes and runs no pipe");
    }
    landed.map(|cluster| (bid, cluster))
}

/// Resolves `target` to `(bid, cluster)`, or `None` for a destination whose
/// line no longer exists. The one fallible step: callers must push the jump
/// entry only *after* this succeeds (`goto_location`'s "no jump entry on
/// failure" contract).
fn resolve_goto_target(
    state: &mut EditorState,
    view: &mut EngineView,
    target: GotoTarget,
) -> Result<Option<(BufferId, ClusterStart)>, String> {
    match target {
        GotoTarget::Wire { uri, pos, encoding } => {
            let path = hume_lsp::uri::uri_to_path(&uri)
                .map_err(|e| format!("cannot open {}: {e}", uri.as_str()))?;
            let (bid, opened) = resolve_or_open(state, view, &path)?;
            let landed = super::wire_to_cluster(state.buffers.get(bid).text(), pos, encoding)
                .map(|(_, cluster)| cluster);
            Ok(settle_opened(state, view, bid, opened, landed))
        }
        GotoTarget::Path {
            path,
            line,
            char_col,
        } => {
            let (bid, opened) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
                view,
                state,
                path.clone(),
            )
            .map_err(|e| format!("{}: {e}", path.display()))?;
            let landed = char_indexed_to_cluster(state, bid, line, char_col);
            Ok(settle_opened(state, view, bid, opened, landed))
        }
    }
}

/// Moves `t`'s own pane to the destination `target` names, recording a jump
/// entry only if it exists and the jump actually lands somewhere else: same
/// "commit point" discipline as `:goto` (`typed_misc.rs`) and buffer
/// switches (`switch_to_buffer_with_jump`). `Ok(Some(handle))` names the
/// pane and the buffer it now shows; `Ok(None)` when the destination's line
/// no longer exists, with nothing moved.
pub(in crate::editor) fn goto_location(
    state: &mut EditorState,
    view: &mut EngineView,
    t: crate::editor::commands::CommandPane,
    target: GotoTarget,
) -> Result<Option<PaneHandle>, String> {
    Ok(resolve_goto_target(state, view, target)?.map(|(bid, pos)| land(state, view, t, bid, pos)))
}

/// `(goto-location! pane (hash 'target pane 'char-offset o))`: moves `t`'s
/// own pane to `char_offset` in `bid`, with the same jump entry as
/// [`goto_location`]. Scheme computed the offset for a live buffer, so one
/// past the buffer's end is an error.
pub(in crate::editor) fn goto_buffer_offset(
    state: &mut EditorState,
    view: &mut EngineView,
    t: crate::editor::commands::CommandPane,
    bid: BufferId,
    char_offset: usize,
) -> Result<PaneHandle, String> {
    let text = state
        .buffers
        .try_get(bid)
        .ok_or_else(|| "no such buffer".to_string())?
        .text();
    let offset = crate::editor::buffer::offsets::checked_char_offset(text, char_offset)?;
    let pos = text.snap(offset);
    Ok(land(state, view, t, bid, pos))
}

/// Jumps `t`'s own pane to `pos` in `bid`, and names the pane and the buffer
/// it now shows.
fn land(
    state: &mut EditorState,
    view: &mut EngineView,
    t: crate::editor::commands::CommandPane,
    bid: BufferId,
    pos: ClusterStart,
) -> PaneHandle {
    let pid = t.pid();
    crate::editor::commands::jump_pane_to(state, view, t, bid, pos);
    PaneHandle::with_pane(bid, pid)
}

/// Answers a server-initiated `workspace/applyEdit` request by actually
/// applying it. Per spec this never fails at the JSON-RPC level: a rejected or
/// malformed edit still gets a 200 response, just with `applied: false`.
///
/// `server_id` is the requesting server (this
/// answers a message that arrived *from* it): its own negotiated
/// encoding is what every position in `params` is counted in, same as
/// any other response it sends.
pub(in crate::editor) fn apply_edit_request_response(
    state: &mut EditorState,
    view: &mut EngineView,
    params: &serde_json::Value,
    server_id: hume_lsp::backend::ServerId,
) -> Result<serde_json::Value, ResponseError> {
    let Some(edit_json) = params.get("edit").cloned() else {
        return Ok(serde_json::json!({
            "applied": false,
            "failureReason": "missing edit",
        }));
    };
    let we: lsp_types::WorkspaceEdit = match serde_json::from_value(edit_json) {
        Ok(we) => we,
        Err(e) => {
            return Ok(serde_json::json!({
                "applied": false,
                "failureReason": format!("malformed edit: {e}"),
            }));
        }
    };
    let Some(encoding) = introspect::server_encoding(&state.lsp, server_id) else {
        return Ok(serde_json::json!({
            "applied": false,
            "failureReason": "lsp server no longer tracked",
        }));
    };
    // Server-initiated, no Steel invocation pane: the focused pane edits the
    // file it shows and every other file is edited for no pane.
    let requester = FocusedPane::current(view).handle(view);
    // No request-time buffer to check staleness against: this answers a
    // server-initiated request, not a Steel round-trip with its own
    // captured generation.
    match apply_workspace_edit(state, view, requester, we, encoding, None) {
        Ok(_summary) => Ok(serde_json::json!({ "applied": true })),
        Err(e) => Ok(serde_json::json!({
            "applied": false,
            "failureReason": e,
        })),
    }
}
