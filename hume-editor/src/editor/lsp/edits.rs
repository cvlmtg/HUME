//! Edit + navigation primitives: `apply-text-edits!`,
//! `apply-workspace-edit!`, `goto-location!`, `selection-spans-full-line?`.
//!
//! Everything but `Editor::apply_edit_request_response` is state/view-only
//! (no `&mut Editor` needed) so those are callable directly from
//! `EditorHostImpl`, the same discipline as the decoration setters. The one
//! exception answers a server-initiated request, which needs the full
//! `apply_workspace_edit` + `detect_pending_languages` pair — it lives here
//! rather than in `drain.rs` because it shares the edit-application path
//! with `apply-workspace-edit!` and belongs next to it.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::grapheme::next_grapheme_boundary;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_lsp::codec::ResponseError;
use hume_rope::position_encoding::{PositionEncoding, wire_to_char};

use super::LspState;
use super::introspect;
use crate::editor::Editor;
use crate::editor::EditorState;
use crate::editor::buffer::Buffer;
use crate::editor::doc_ops;
use crate::editor::pane_state;

/// Shared precondition check for every edit entry point in this module: the
/// buffer exists, is writable, and hasn't moved since the caller computed its
/// positions against it. One definition so `build_edit_changeset` and
/// `CompletionSession::accept` (which needs the same guard but isn't
/// building from wire `TextEdit`s) can't drift apart.
pub(crate) fn checked_buffer(
    state: &EditorState,
    bid: BufferId,
    expect_gen: Option<u64>,
) -> Result<&Buffer, String> {
    let Some(buf) = state.buffers.try_get(bid) else {
        return Err("no such buffer".to_string());
    };
    if buf.is_read_only() {
        return Err("buffer is read-only".to_string());
    }
    if let Some(expected_gen) = expect_gen
        && buf.text_gen != expected_gen
    {
        return Err("buffer changed since these edits were computed".to_string());
    }
    Ok(buf)
}

fn one_of_to_text_edit(
    oe: lsp_types::OneOf<lsp_types::TextEdit, lsp_types::AnnotatedTextEdit>,
) -> lsp_types::TextEdit {
    match oe {
        lsp_types::OneOf::Left(te) => te,
        lsp_types::OneOf::Right(ate) => ate.text_edit,
    }
}

/// Converts `edits` (wire positions, any order) into one composite
/// `ChangeSet`: positions are resolved to char offsets, stable-sorted
/// ascending by start (tie order = `edits`' own array order, per spec),
/// checked for overlap (adjacent edits are fine; a shared boundary is not),
/// then walked forward once to build the `ChangeSet`. Read-only, no
/// mutation: `apply_workspace_edit`'s "validate all, then apply all" needs
/// to build every file's changeset before committing any of them.
fn build_edit_changeset(
    state: &EditorState,
    lsp: &LspState,
    bid: BufferId,
    edits: &[lsp_types::TextEdit],
    expect_gen: Option<u64>,
) -> Result<ChangeSet, String> {
    let buf = checked_buffer(state, bid, expect_gen)?;
    if edits.is_empty() {
        return Err("no edits given".to_string());
    }
    let encoding = introspect::encoding_for_buffer(state, lsp, bid);
    let rope = buf.text().rope();
    // Stable ascending sort by start — two edits at the same position keep
    // `edits`' own array order (per spec, the array's order defines the
    // order same-position edits apply in; a descending sort followed by
    // `.reverse()` would keep ties in *original* order through the sort but
    // then flip that tie order via the whole-`Vec` reverse, applying them
    // backwards).
    let char_edits: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            let (start, end) = super::wire_range_to_chars(rope, &e.range, encoding);
            (start, end, e.new_text.as_str())
        })
        .collect();
    build_changeset_from_char_edits(rope.len_chars(), char_edits)
}

/// Shared tail for [`build_edit_changeset`] (wire positions, converted to
/// char offsets above) and the completion-resolve path (already char
/// offsets, mapped forward through the accept edit) — sort, overlap-check,
/// then walk once to build the `ChangeSet`. `edits` need not arrive sorted;
/// `new_text` borrows from the caller's own edit list, so this never
/// allocates a copy of the replacement text.
fn build_changeset_from_char_edits(
    len_before: usize,
    mut char_edits: Vec<(usize, usize, &str)>,
) -> Result<ChangeSet, String> {
    if let Some((start, end, _)) = char_edits.iter().find(|&&(start, end, _)| end < start) {
        return Err(format!(
            "text edit has a reversed range (end {end} before start {start})"
        ));
    }
    char_edits.sort_by_key(|e| e.0);
    for w in char_edits.windows(2) {
        if w[1].0 < w[0].1 {
            return Err("text edits overlap".to_string());
        }
    }

    let mut b = ChangeSetBuilder::new(len_before);
    for (start, end, text) in &char_edits {
        b.retain(start - b.old_pos());
        b.delete(end - start);
        b.insert(text);
    }
    b.retain_rest();
    Ok(b.finish())
}

/// Applies a pre-built `ChangeSet` to `bid` as one undo step, through the
/// same chokepoint every native edit command uses
/// (`doc_ops::apply_doc_edit` — selection propagation, syntax reparse,
/// queued LSP `didChange`, all for free). `bid` need not be the buffer
/// shown in the focused pane — `pane_state::ensure` seeds a (possibly
/// invisible) selection entry for it first, matching how any buffer opened
/// in the background already gets one. Returns the applied `ChangeSet`
/// (cloned before the move into `apply_doc_edit`'s closure) so a caller that
/// needs to map further positions through this exact edit — completion's
/// resolve path, mapping a *pre*-accept response's positions forward — can
/// do so without rebuilding it.
fn commit_changeset(state: &mut EditorState, bid: BufferId, cs: ChangeSet) -> ChangeSet {
    pane_state::ensure(
        &mut state.panes.state,
        &state.buffers,
        state.focused_pane_id,
        bid,
    );
    let cs_for_return = cs.clone();
    doc_ops::apply_doc_edit(
        &mut state.buffers,
        &state.config.decorations,
        &mut state.panes.state,
        state.focused_pane_id,
        bid,
        move |text, mut sels| {
            let buf_pre = text.clone();
            sels.translate_in_place(&cs, &buf_pre);
            let new_text = cs
                .apply(&text)
                .expect("cs built from this buffer's own rope, just above");
            (new_text, sels, cs)
        },
    );
    cs_for_return
}

/// `(apply-text-edits! bid edits #:expect-generation gen)`.
pub(crate) fn apply_text_edits(
    state: &mut EditorState,
    lsp: &LspState,
    bid: BufferId,
    edits: Vec<lsp_types::TextEdit>,
    expect_gen: Option<u64>,
) -> Result<(), String> {
    apply_text_edits_returning_cs(state, lsp, bid, edits, expect_gen)?;
    Ok(())
}

/// Same as [`apply_text_edits`] but hands back the applied `ChangeSet` —
/// completion's accept path needs it to map a subsequent
/// `completionItem/resolve` response's positions (computed against the
/// pre-accept document) forward onto the buffer as it stands after this
/// edit landed.
pub(crate) fn apply_text_edits_returning_cs(
    state: &mut EditorState,
    lsp: &LspState,
    bid: BufferId,
    edits: Vec<lsp_types::TextEdit>,
    expect_gen: Option<u64>,
) -> Result<ChangeSet, String> {
    let cs = build_edit_changeset(state, lsp, bid, &edits, expect_gen)?;
    Ok(commit_changeset(state, bid, cs))
}

/// Decodes `edits` (wire positions, computed by the server against the
/// document as it stood at `rope_at`) into char-offset `(start, end, text)`
/// triples valid against the document `cs_forward` transforms `rope_at`
/// into — exact position tracking through every edit `cs_forward` composes,
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
/// Returns an empty `Vec` (not an error) when `edits` is empty — matches
/// `apply-text-edits!`'s convention of erroring on an empty list only when
/// the caller has no legitimate empty-response case; both callers here do
/// (no `additionalTextEdits` at all is normal).
pub(crate) fn build_edits_from_earlier_document<'a>(
    rope_at: &ropey::Rope,
    cs_forward: &ChangeSet,
    encoding: PositionEncoding,
    edits: &'a [lsp_types::TextEdit],
) -> Result<Vec<(usize, usize, &'a str)>, String> {
    if edits.is_empty() {
        return Ok(Vec::new());
    }
    let mut ranges: Vec<(usize, usize)> = edits
        .iter()
        .map(|e| super::wire_range_to_chars(rope_at, &e.range, encoding))
        .collect();
    if let Some(&(start, end)) = ranges.iter().find(|&&(start, end)| end < start) {
        return Err(format!(
            "text edit has a reversed range (end {end} before start {start})"
        ));
    }
    // `map_ranges` requires sorted-by-start input; sort the (range, text)
    // pairing together so the mapped range still lines up with its own text
    // afterward.
    let mut indexed: Vec<usize> = (0..ranges.len()).collect();
    indexed.sort_by_key(|&i| ranges[i].0);
    ranges.sort_by_key(|&(start, _)| start);
    cs_forward.map_ranges(&mut ranges);

    Ok(indexed
        .into_iter()
        .zip(ranges)
        .map(|(orig_i, (start, end))| (start, end, edits[orig_i].new_text.as_str()))
        .collect())
}

/// Commits pre-computed char-offset edits (from
/// [`build_edits_from_earlier_document`] or any other char-space source)
/// against `bid`'s *current* text as one `ChangeSet` — validates
/// overlap/reversed-range across the batch itself (via
/// [`build_changeset_from_char_edits`]) immediately before mutating.
/// `Ok(None)` for an empty batch (nothing to commit); `Ok(Some(cs))`
/// otherwise, so a caller composing this into a larger changeset doesn't need
/// its own empty-batch branch.
pub(crate) fn commit_char_edits(
    state: &mut EditorState,
    bid: BufferId,
    char_edits: Vec<(usize, usize, &str)>,
) -> Result<Option<ChangeSet>, String> {
    if char_edits.is_empty() {
        return Ok(None);
    }
    let buf = checked_buffer(state, bid, None)?;
    let len_before = buf.text().rope().len_chars();
    let cs = build_changeset_from_char_edits(len_before, char_edits)?;
    Ok(Some(commit_changeset(state, bid, cs)))
}

pub(crate) struct WorkspaceEditSummary {
    pub(crate) buffers_modified: usize,
}

/// One resolved file entry: its URI, plain `TextEdit`s (annotations
/// stripped — HUME has no change-annotation UI), and the version to
/// gen-check against, if the edit came from a versioned `documentChanges`
/// entry.
type EditEntry = (lsp_types::Uri, Vec<lsp_types::TextEdit>, Option<i32>);

/// `documentChanges` takes precedence over `changes` when both are present
/// (LSP spec). `DocumentChangeOperation::Op` (create/rename/delete file) has
/// no HUME equivalent — errors rather than silently dropping a file
/// operation the caller expected to happen.
fn collect_edit_entries(we: lsp_types::WorkspaceEdit) -> Result<Vec<EditEntry>, String> {
    if let Some(doc_changes) = we.document_changes {
        match doc_changes {
            lsp_types::DocumentChanges::Edits(edits) => Ok(edits
                .into_iter()
                .map(|tde| {
                    let version = tde.text_document.version;
                    let edits = tde.edits.into_iter().map(one_of_to_text_edit).collect();
                    (tde.text_document.uri, edits, version)
                })
                .collect()),
            lsp_types::DocumentChanges::Operations(ops) => ops
                .into_iter()
                .map(|op| match op {
                    lsp_types::DocumentChangeOperation::Edit(tde) => {
                        let version = tde.text_document.version;
                        let edits = tde.edits.into_iter().map(one_of_to_text_edit).collect();
                        Ok((tde.text_document.uri, edits, version))
                    }
                    lsp_types::DocumentChangeOperation::Op(_) => Err(
                        "workspace edit contains an unsupported file operation (create/rename/delete)"
                            .to_string(),
                    ),
                })
                .collect::<Result<Vec<_>, String>>(),
        }
    } else if let Some(changes) = we.changes {
        Ok(changes
            .into_iter()
            .map(|(uri, edits)| (uri, edits, None))
            .collect())
    } else {
        Ok(Vec::new())
    }
}

fn resolve_or_open(
    state: &mut EditorState,
    view: &mut EngineView,
    path: &std::path::Path,
) -> Result<BufferId, String> {
    // `resolve_buffer_path`, not a hard `canonicalize`: a server-driven
    // rename or workspace edit may target a file that doesn't exist yet —
    // openable here exactly like `:e` on a missing path (see
    // `Buffer::from_file_or_new`).
    let resolved = Editor::resolve_buffer_path(path, &state.cwd);
    let (bid, is_new) =
        crate::editor::buffer::lifecycle::open_or_dedup_and_notify(view, state, &resolved)
            .map_err(|e| format!("{}: {e}", resolved.display()))?;
    if is_new {
        let display = hume_platform::path::display_form(&hume_platform::path::absolute_unresolved(
            path, &state.cwd,
        ));
        state.buffers.get_mut(bid).set_display_path(Some(display));
    }
    Ok(bid)
}

/// `(apply-workspace-edit! edit)` — validates and builds every file's
/// changeset first (opening unopened files as buffers along the way), and
/// only commits any of them once every file has passed: a bad edit in file
/// 3 of 5 must leave files 1 and 2 untouched.
pub(crate) fn apply_workspace_edit(
    state: &mut EditorState,
    view: &mut EngineView,
    lsp: &LspState,
    we: lsp_types::WorkspaceEdit,
) -> Result<WorkspaceEditSummary, String> {
    let entries = collect_edit_entries(we)?;
    let mut planned: Vec<(BufferId, ChangeSet)> = Vec::with_capacity(entries.len());
    for (uri, edits, version) in entries {
        let path = hume_lsp::uri::uri_to_path(&uri).map_err(|e| format!("bad uri: {e:?}"))?;
        let bid = resolve_or_open(state, view, &path)?;
        // Each file's changeset is built against `state`'s *current* text —
        // a second entry for the same file would build a changeset that's
        // valid against that same original text, but `commit_changeset`
        // applies entries one at a time against whatever the buffer holds
        // *after* the previous commit. Position-based ops don't necessarily
        // fail to apply against the wrong text (they can silently produce
        // corrupted content instead of erroring) — so this must be rejected
        // before any commit, not left to a downstream length coincidence.
        let display = || {
            state
                .buffers
                .get(bid)
                .display_path()
                .map(str::to_owned)
                .unwrap_or_else(|| path.display().to_string())
        };
        if planned.iter().any(|(planned_bid, _)| *planned_bid == bid) {
            return Err(format!(
                "{}: workspace edit contains more than one entry for this file",
                display()
            ));
        }
        let expect_gen = version.map(|v| v as u64);
        let cs = build_edit_changeset(state, lsp, bid, &edits, expect_gen)
            .map_err(|e| format!("{}: {e}", display()))?;
        planned.push((bid, cs));
    }
    let buffers_modified = planned.len();
    for (bid, cs) in planned {
        commit_changeset(state, bid, cs);
    }
    Ok(WorkspaceEditSummary { buffers_modified })
}

/// `(goto-location! target)` — either shape:
/// - a raw `Location`/`LocationLink` hashmap (wire positions, converted with
///   `focused_bid`'s attached server's encoding — the server that produced
///   this location negotiated that encoding for every response it sends,
///   regardless of which file the location points into);
/// - `(list target line char-col)`, already char-indexed — `target` is a path
///   string, a `file://` URI string, or a `bid`.
pub(crate) enum GotoTarget {
    Wire {
        uri: lsp_types::Uri,
        line: usize,
        character: usize,
    },
    Path {
        path_or_uri: String,
        line: usize,
        char_col: usize,
    },
    Buffer {
        bid: BufferId,
        line: usize,
        char_col: usize,
    },
}

/// Clamps a char-indexed `(line, char_col)` pair to a valid char offset in
/// `bid`. `line` clamps to the ropey-domain last line here (a scripted
/// target can address the buffer's own trailing phantom line); the
/// char_col clamp and grapheme snap are `place_char_column`'s, which lands a
/// past-the-end column on the line's last content character rather than on
/// its `\n`.
fn char_indexed_to_char_pos(
    state: &EditorState,
    bid: BufferId,
    line: usize,
    char_col: usize,
) -> usize {
    let buf = state.buffers.get(bid);
    let text = buf.text();
    let line = line.min(text.last_ropey_line());
    hume_editing::lines::place_char_column(text, line, char_col)
}

/// A bare path string and a `file://` URI string both name shape 2's
/// `target`; try URI first since a `file://` string doesn't round-trip
/// through path expansion cleanly.
fn resolve_path_or_uri(
    state: &mut EditorState,
    view: &mut EngineView,
    path_or_uri: &str,
) -> Result<BufferId, String> {
    if let Ok(uri) = path_or_uri.parse::<lsp_types::Uri>()
        && uri.scheme().is_some_and(|s| s.eq_lowercase("file"))
    {
        let path = hume_lsp::uri::uri_to_path(&uri).map_err(|e| format!("bad uri: {e:?}"))?;
        return resolve_or_open(state, view, &path);
    }
    let expanded = hume_platform::path::expand(path_or_uri);
    resolve_or_open(state, view, std::path::Path::new(expanded.as_ref()))
}

/// Resolves `target` to `(bid, char_pos)` — the one fallible step. Callers
/// must push the jump entry only *after* this succeeds (`goto_location`'s
/// "no jump entry on failure" contract).
fn resolve_goto_target(
    state: &mut EditorState,
    view: &mut EngineView,
    lsp: &LspState,
    focused_bid: BufferId,
    target: GotoTarget,
) -> Result<(BufferId, usize), String> {
    match target {
        GotoTarget::Wire {
            uri,
            line,
            character,
        } => {
            let path = hume_lsp::uri::uri_to_path(&uri).map_err(|e| format!("bad uri: {e:?}"))?;
            let bid = resolve_or_open(state, view, &path)?;
            let encoding = introspect::encoding_for_buffer(state, lsp, focused_bid);
            let rope = state.buffers.get(bid).text().rope();
            Ok((bid, wire_to_char(rope, line, character, encoding)))
        }
        GotoTarget::Path {
            path_or_uri,
            line,
            char_col,
        } => {
            let bid = resolve_path_or_uri(state, view, &path_or_uri)?;
            Ok((bid, char_indexed_to_char_pos(state, bid, line, char_col)))
        }
        GotoTarget::Buffer {
            bid,
            line,
            char_col,
        } => {
            if state.buffers.try_get(bid).is_none() {
                return Err("no such buffer".to_string());
            }
            Ok((bid, char_indexed_to_char_pos(state, bid, line, char_col)))
        }
    }
}

/// Moves the focused pane to `(bid, char_pos)`, recording a jump entry only
/// if resolution succeeded — same "commit point" discipline as `:goto`
/// (`typed_misc.rs`) and buffer switches (`switch_to_buffer_with_jump`).
pub(crate) fn goto_location(
    state: &mut EditorState,
    view: &mut EngineView,
    lsp: &LspState,
    target: GotoTarget,
) -> Result<(), String> {
    let focused_bid = crate::editor::commands::focused_buffer_id(state, view);
    let (bid, char_pos) = resolve_goto_target(state, view, lsp, focused_bid, target)?;
    // Every path above can legitimately return `len_chars()` (e.g. a wire
    // line past EOF, or a char-indexed target on the trailing structural
    // line, both clamp to that line's start = len_chars()) — but cursors
    // must satisfy `head < len_chars()`. Clamp to the last char (the
    // buffer's own trailing `\n`, always present and always its own
    // grapheme boundary, so no snap is needed).
    let len_chars = state.buffers.get(bid).text().rope().len_chars();
    let char_pos = char_pos.min(len_chars.saturating_sub(1));

    let entry = crate::editor::commands::current_jump_entry(state, view);
    state.panes.jumps[state.focused_pane_id].push(entry);

    let pid = state.focused_pane_id;
    crate::editor::buffer::lifecycle::switch_pane_to_buffer(
        view,
        &state.buffers,
        &mut state.panes.state,
        pid,
        bid,
    );
    pane_state::ensure(&mut state.panes.state, &state.buffers, pid, bid);
    state.panes.state[pid][bid].selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(char_pos),
    );

    let height = view.panes[pid].viewport.height as usize;
    let rope = state.buffers.get(bid).text().rope();
    let cursor_line = rope.char_to_line(char_pos);
    view.panes[pid].viewport.top_line = cursor_line.saturating_sub(height / 2);

    Ok(())
}

/// `(selection-spans-full-line? bid)` — the primary selection covers exactly
/// one line, start to (and including) its trailing newline.
pub(crate) fn selection_spans_full_line(state: &EditorState, bid: BufferId) -> bool {
    let Some(buf) = state.buffers.try_get(bid) else {
        return false;
    };
    let pid = state.focused_pane_id;
    let Some(pbs) = state
        .panes
        .state
        .get(pid)
        .and_then(|by_buf| by_buf.get(bid))
    else {
        return false;
    };
    let text = buf.text();
    let sel = pbs.selections.primary();
    let start = sel.start();
    let end_exclusive = next_grapheme_boundary(text, sel.end());
    let line = text.char_to_line(start);
    let line_start = text.line_to_char(line);
    let line_end = hume_editing::lines::line_end_exclusive(text, line);
    start == line_start && end_exclusive == line_end
}

impl Editor {
    /// Answers a server-initiated `workspace/applyEdit` request by actually
    /// applying it. Per spec this never fails at the JSON-RPC level: a rejected or
    /// malformed edit still gets a 200 response, just with `applied: false`.
    pub(in crate::editor) fn apply_edit_request_response(
        &mut self,
        params: &serde_json::Value,
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
        let result = apply_workspace_edit(&mut self.state, &mut self.view, &self.lsp, we);
        // Drain regardless of outcome: `apply_workspace_edit`'s contract is
        // "validate all, then apply all", but it opens buffers as it *validates*
        // each entry (`edits.rs`'s `resolve_or_open` calls), so a failure on
        // entry 3 of 5 still leaves entries 1-2's buffers open and queued here.
        self.detect_pending_languages();
        match result {
            Ok(_summary) => Ok(serde_json::json!({ "applied": true })),
            Err(e) => Ok(serde_json::json!({
                "applied": false,
                "failureReason": e,
            })),
        }
    }
}
