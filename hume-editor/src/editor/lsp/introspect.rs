//! The introspection surface: capabilities, server status, generation, and
//! ready-made wire-position params. All read-only: no queueing, unlike
//! request/notify (which must defer to the eval-result drain boundary
//! because they mutate the transport). These run through `EditorHostImpl`
//! directly since a Steel caller needs the value back inline.

use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_lsp::backend::ServerId;

use super::LspState;
use super::diagnostics::DiagSeverity;
use super::registry::LanguageName;
use crate::editor::Editor;
use crate::editor::EditorState;

/// Resolves `bid`'s own attached server to a running `ServerId`. Never a
/// fallback to whichever buffer happens to be focused when this runs, so a caller
/// resolving a follow-up request from inside a response callback gets the
/// buffer the original request was about, not one a user's intervening
/// keystrokes moved focus to.
///
/// Errors loudly on a Crashed (or otherwise untracked) server rather than
/// resolving to it: its sends are silently dropped (`send_or_queue`), so a
/// caller would otherwise learn of the problem only as a generic timeout at
/// the request's deadline. `Starting` still resolves: `send_or_queue`'s
/// Starting-queue correctly defers the send until the handshake completes.
pub(super) fn resolve_server_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    bid: BufferId,
) -> Result<ServerId, String> {
    let sid = state
        .buffers
        .try_get(bid)
        .ok_or_else(|| format!("invalid buffer id {bid:?}"))?
        .lsp_server
        .ok_or_else(|| "no LSP server attached to this buffer".to_string())?;
    match lsp.servers.get(&sid).map(|e| e.client.state()) {
        Some(hume_lsp::client::ServerState::Starting | hume_lsp::client::ServerState::Running) => {
            Ok(sid) // send_or_queue handles Starting's deferred send correctly
        }
        Some(hume_lsp::client::ServerState::Crashed) => {
            Err("lsp server crashed (run :lsp-restart)".to_string())
        }
        Some(hume_lsp::client::ServerState::Dead) | None => Err("lsp server stopped".to_string()),
    }
}

/// The registered language for `server_id`: reverse of the
/// `(language, root) -> ServerId` lookup `lsp/registration.scm` uses to
/// attach a buffer to a server.
pub(super) fn server_language(lsp: &LspState, server_id: ServerId) -> Option<LanguageName> {
    lsp.servers.get(&server_id)?.language.clone()
}

/// Whether `server` advertises `completionProvider.resolveProvider`, the
/// gate `BufferSession::accept`'s resolve round trip reads. A narrow reader rather than
/// widening `LspState.servers`/`ServerEntry.client` themselves: the
/// completion store lives outside this module now, and one bool is all it
/// needs.
pub(in crate::editor) fn completion_resolve_provider(lsp: &LspState, server: ServerId) -> bool {
    lsp.servers
        .get(&server)
        .and_then(|e| e.client.capabilities_json())
        .and_then(|caps| caps.get("completionProvider"))
        .and_then(|cp| cp.get("resolveProvider"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// The server's raw wire capabilities. See `LspClient::capabilities_json`'s
/// doc comment for why this, not the typed decode, is what
/// `(lsp-capabilities …)` must hand to Steel. `Arc`-wrapped: this clone is
/// just a refcount bump, not a deep copy of the capabilities blob.
pub(in crate::editor) fn capabilities(
    state: &EditorState,
    lsp: &LspState,
    bid: BufferId,
) -> Option<std::sync::Arc<serde_json::Value>> {
    let sid = resolve_server_for_buffer(state, lsp, bid).ok()?;
    lsp.servers.get(&sid)?.client.capabilities_json().cloned()
}

/// One entry per running (language, root) server: `:lsp-status`'s data in
/// structured form.
pub(in crate::editor) fn server_status(
    lsp: &LspState,
) -> Vec<hume_scripting::LspServerStatusEntry> {
    lsp.servers
        .values()
        .filter_map(|e| {
            let language = e.language.clone()?;
            Some(hume_scripting::LspServerStatusEntry {
                language,
                root: e.client.root().to_path_buf(),
                state: e.client.state().name(),
                pending: e.client.pending_count(),
            })
        })
        .collect()
}

/// The registered language for the server attached to buffer `id`.
pub(in crate::editor) fn server_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> Option<LanguageName> {
    let sid = state.buffers.try_get(id)?.lsp_server?;
    server_language(lsp, sid)
}

/// A buffer's attached server's loading state. Drives the statusline's
/// loading spinner (`statusline::elements::diagnostics`).
pub(crate) enum LspActivity {
    /// No attached server, a `Running` server with no progress task in
    /// flight, or a `Crashed`/`Dead` one: nothing to animate.
    Idle,
    /// Mid `initialize` handshake.
    Starting,
    /// A `$/progress` task (indexing, loading, ...) is in flight: the most
    /// recently begun one, if the server is running more than one. Carries
    /// no title: the statusline only shows the spinner + percentage
    /// (`statusline::elements::diagnostics`); the underlying task's
    /// title is reachable via `LspState::progress_title_for_test` for tests
    /// that need to assert the begin/report merge machine.
    Progress { percentage: Option<u32> },
}

/// `id`'s attached server's current [`LspActivity`].
pub(crate) fn activity(state: &EditorState, lsp: &LspState, id: BufferId) -> LspActivity {
    let Some(sid) = state.buffers.try_get(id).and_then(|b| b.lsp_server) else {
        return LspActivity::Idle;
    };
    let Some(entry) = lsp.servers.get(&sid) else {
        return LspActivity::Idle;
    };
    if entry.client.state() == hume_lsp::client::ServerState::Starting {
        return LspActivity::Starting;
    }
    match entry.progress.last() {
        Some((_, task)) => LspActivity::Progress {
            percentage: task.percentage,
        },
        None => LspActivity::Idle,
    }
}

/// Whether `language` currently has a `register-lsp-server!` config:
/// registered, not necessarily attached/running. Distinguishes "no server
/// registered" from "registered but still starting", which
/// `server_for_buffer` (attachment, not registration) can't tell apart.
pub(in crate::editor) fn registered_for_language(lsp: &LspState, language: &str) -> bool {
    lsp.configs.contains_key(language)
}

/// Shared setup for both params builders: the buffer's URI and its attached
/// server's negotiated encoding. `None` if `id` has no path or no attached
/// (tracked) server.
fn uri_and_encoding(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> Option<(String, hume_rope::position_encoding::PositionEncoding)> {
    let buf = state.buffers.try_get(id)?;
    let path = buf.path()?;
    let sid = buf.lsp_server?;
    let entry = lsp.servers.get(&sid)?;
    let uri = hume_lsp::uri::path_to_uri(path).ok()?;
    Some((uri.as_str().to_string(), entry.client.encoding()))
}

/// Ready-made `{"textDocument" {"uri"} "position" {"line" "character"}}`
/// params from the primary cursor head in `t`'s own pane. `None` only when
/// `t`'s buffer has no path or no attached server. `t` is already resolved
/// (see `commands::CommandPane::resolve`), so there is no "not shown" case left.
pub(in crate::editor) fn position_params(
    state: &EditorState,
    view: &EngineView,
    lsp: &LspState,
    t: crate::editor::commands::CommandPane,
) -> Option<serde_json::Value> {
    let id = t.bid(view);
    let head = crate::editor::commands::pane_view(state, view, t)
        .primary()
        .head();
    offset_params(state, lsp, id, head.offset())
}

/// [`position_params`] for `offset` of `id`'s live text instead of a pane's
/// cursor. `None` when `id` is closed, has no path, or has no attached
/// server.
pub(in crate::editor) fn offset_params(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
    offset: hume_rope::offset::CharOffset,
) -> Option<serde_json::Value> {
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let text = state.buffers.try_get(id)?.text();
    let pos = hume_rope::position_encoding::char_to_wire(text.rope(), offset, encoding);
    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "position": hume_lsp::position::to_json_position(pos),
    }))
}

/// The negotiated encoding of a specific running server: the counterpart
/// for a caller that already has a `ServerId` in hand instead of resolving
/// one from a `BufferId` (a server-initiated request, or a response being
/// tagged at dispatch time). `None` if the server is no longer tracked.
pub(in crate::editor) fn server_encoding(
    lsp: &LspState,
    sid: ServerId,
) -> Option<hume_rope::position_encoding::PositionEncoding> {
    lsp.servers.get(&sid).map(|e| e.client.encoding())
}

/// Wire `(line, character)` → char offset, decoded in `encoding`, for
/// `lsp-range->offsets`. `None` if `id` is unknown. `encoding` is the
/// caller's own resolve: the tagged producing-server encoding of the
/// response the position was read out of (`JsonHandle::position_encoding`),
/// never `id`'s *currently* attached server: the two can diverge (a restart
/// renegotiates a different encoding, a detach leaves none), and the tag is
/// always the encoding the position was actually written in. No `LspState`
/// lookup needed at all, for the same reasoning as [`label_slice`]'s own `encoding`
/// parameter, below.
///
/// Clamps rather than errors on an out-of-range `line`/`character`, same as
/// `wire_to_char` itself: a range's `end` legitimately lands exactly at
/// the buffer's `len_chars()` (`set-extra-highlights!`'s `validate_range`
/// accepts that boundary), so this must not reject it. Point-anchored
/// callers want the opposite; see [`wire_point_to_char_for_buffer`].
pub(in crate::editor) fn wire_to_char_for_buffer(
    state: &EditorState,
    id: BufferId,
    pos: hume_rope::position_encoding::WirePos,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> Option<hume_rope::offset::CharOffset> {
    let rope = state.buffers.try_get(id)?.text().rope();
    Some(hume_rope::position_encoding::wire_to_char(
        rope, pos, encoding,
    ))
}

/// Wire `(line, character)` → char offset, for `lsp-position->offset`.
/// Same conversion as [`wire_to_char_for_buffer`], but refuses (`None`)
/// when the result lands at `len_chars()`: the position `wire_to_char`
/// clamps a past-end `line` onto (the buffer's trailing phantom line, every
/// buffer ending with a structural `\n`). A point-anchored decoration
/// setter (`set-inlay-hints!`'s `validate_offset`) rejects that offset
/// outright: handing it back here would let a single stale server response
/// (a request that raced an edit) fail the caller's *entire* hint batch
/// (`collect::<Result<Vec<_>, _>>` in `host_impl.rs`) instead of just being
/// filtered out, one entry, by the caller's own `#f` check.
pub(in crate::editor) fn wire_point_to_char_for_buffer(
    state: &EditorState,
    id: BufferId,
    pos: hume_rope::position_encoding::WirePos,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> Option<hume_rope::offset::CharOffset> {
    let offset = wire_to_char_for_buffer(state, id, pos, encoding)?;
    let text = state.buffers.try_get(id)?.text();
    (offset < text.end()).then_some(offset)
}

/// `label` sliced by a `ParameterInformation.label` `[start, end)` wire
/// offset pair, for `lsp-label-offsets->text`. The one place a wire offset
/// indexes a *server-authored string* rather than a document: the offsets
/// address `SignatureInformation.label`, which never reaches a buffer, so
/// none of the `bid`-anchored converters above fit. `encoding` is the
/// caller's own resolve (the `offsets` handle's tagged producing-server
/// encoding). This function needs no buffer or `LspState` lookup at all.
pub(in crate::editor) fn label_slice(
    label: &str,
    start: usize,
    end: usize,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> String {
    let range =
        hume_rope::position_encoding::wire_offsets_to_byte_range(label, start, end, encoding);
    label[range].to_string()
}

/// `(diagnostics-for-buffer bid #:severity floor #:range (start . end))`:
/// decoded, filtered, capped-at-1000 hashmaps. `start`/`end`
/// are char offsets; `line`/`char-col` are the char-indexed start position,
/// ready for `goto-location!` shape 2, an *addressing* unit, exact and
/// lossless. `grapheme-col` is the same position as a grapheme column
/// instead, for *display*: the one unit every HUME surface (statusline,
/// diagnostics, LSP location lists) shows the user; never render `char-col`
/// directly. `end-line` is the range's *end* clamped and converted the same
/// way `line` is. The diagnostics plugin's gutter-sign pass expands
/// `[line, end-line]` inclusive to mark every line a multi-line diagnostic
/// touches. `severity-rank` is `DiagSeverity`'s own `Ord` discriminant (`0`
/// for error, counting up to `3` for hint) alongside the `severity` string,
/// so a caller compares severities by this rather than re-deriving the same
/// order from the string. Errors loudly on an unknown `#:severity` name
/// (e.g. `'warn` typoed for `'warning`) rather than silently returning
/// nothing that qualifies.
///
/// With no `#:severity`, defaults to `lsp.diagnostics-severity-floor`, the
/// same floor `update_highlight_providers` applies to underlines, so a
/// caller (e.g. the diagnostics plugin's EOL summary and gutter signs)
/// agrees with what's on screen unless it explicitly asks for a different
/// cut.
pub(in crate::editor) fn diagnostics_for_buffer(
    state: &EditorState,
    bid: BufferId,
    severity_floor: Option<&str>,
    range: Option<hume_rope::offset::ExclusiveRange<hume_rope::offset::CharOffset>>,
) -> Result<Vec<hume_scripting::host::DiagnosticEntry>, String> {
    const CAP: usize = 1000;
    let floor = match severity_floor.map(str::parse::<DiagSeverity>) {
        None => state.settings.lsp_diagnostics_severity_floor,
        Some(Ok(f)) => f,
        Some(Err(e)) => return Err(e),
    };
    let Some(text) = state.buffers.try_get(bid).map(|b| b.text()) else {
        return Ok(Vec::new());
    };
    // `text.end()`, not a `usize::MAX` sentinel: the buffer's own exclusive
    // end bound is the honest "whole buffer" default, and only becomes
    // available once `text` is in hand. This is why the default is
    // resolved here rather than at the Host seam that converts `range` into
    // this type.
    let range = range.unwrap_or_else(|| {
        hume_rope::offset::ExclusiveRange::new(hume_rope::offset::CharOffset::new(0), text.end())
    });

    let entries = state
        .buffer_positions
        .diagnostics
        .for_range(bid, range, floor)
        .take(CAP)
        .map(|d| {
            // Clamped to the last *content* char, not `len_chars()`: a
            // server can report a diagnostic anchored at end-of-file, one
            // past the buffer's last real char, and `len_chars()` itself
            // resolves to the buffer's trailing phantom empty line (every
            // buffer ends with a structural `\n`; see
            // `hume-editor/src/editor/host_impl.rs`'s `line_start_offset`).
            // Landing there instead of the buffer's last content line would
            // hand plugins a `line` that later fails the fail-fast bound
            // check every decoration setter now enforces.
            let last_content_char = text.last_content_char();
            let clamped_start = d.start.min(last_content_char);
            let line = text.char_to_line(clamped_start);
            let char_col = hume_editing::lines::char_col_in_line(text, line, clamped_start);
            let grapheme_col =
                hume_editing::grapheme::grapheme_col_in_line(text, line, clamped_start);
            // `end-line` mirrors `line`'s clamp so a range that reaches (or
            // overshoots) end-of-file still names the buffer's last content
            // line rather than the phantom trailing one. The gutter-sign
            // plugin expands `[line, end-line]` inclusive to mark every line
            // a multi-line diagnostic touches.
            let end_line = text.char_to_line(d.end.retreat_saturating(1).min(last_content_char));
            hume_scripting::host::DiagnosticEntry {
                start: d.start.index(),
                end: d.end.index(),
                line: line.index(),
                end_line: end_line.index(),
                char_col: char_col.index(),
                grapheme_col: grapheme_col.index(),
                severity: d.severity.to_string(),
                severity_rank: d.severity as u8,
                message: d.message.clone(),
                code: d.code.clone(),
                source: d.source.clone(),
                raw: std::sync::Arc::clone(&d.raw),
                encoding: d.encoding,
            }
        })
        .collect();
    Ok(entries)
}

/// `(diagnostic-counts bid)` → `(errors, warnings)`.
pub(crate) fn diagnostic_counts(state: &EditorState, bid: BufferId) -> (usize, usize) {
    state.buffer_positions.diagnostics.counts(bid)
}

/// Current animation frame for the statusline loading spinner.
///
/// Here rather than on `Editor` because `LspState::spinner` is private to
/// `mod lsp`, and the statusline reads this holding only an `&LspState`.
pub(crate) fn spinner_frame(lsp: &LspState) -> usize {
    lsp.spinner.frame
}

/// `line`/`character` clamped into `text`'s addressable range and converted
/// to a grapheme column. `None` when `line` names no real content, rather
/// than silently reporting a column under a `line` that doesn't match it.
///
/// The bound is the last *content* line, so a server's past-end response and
/// the buffer's own phantom trailing line (the one the structural `\n`
/// creates) both return `None`. Clamping to the ropey domain instead would
/// admit the phantom line and report column 1 of a line that has no
/// characters: a drawer row pointing one line past the file's end.
fn wire_pos_to_grapheme_col(
    text: &hume_editing::text::BufferText,
    pos: hume_rope::position_encoding::WirePos,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> Option<hume_rope::column::GraphemeCol> {
    if pos.line > text.last_content_line().index() {
        return None;
    }
    let target = super::wire_to_cluster(text, pos, encoding);
    // Trusted narrow: the bound check above already confirmed `line` names a
    // real content line.
    let line = hume_rope::line::ContentLine::new(pos.line);
    Some(hume_editing::grapheme::grapheme_col_in_line(
        text,
        line,
        target.offset(),
    ))
}

/// Filesystem path, wire line, and column for a batch of raw
/// `Location`/`LocationLink` LSP locations, backing
/// `lsp-locations->display-parts` (goto/references drawer rows).
///
/// Each location is decoded once through [`hume_lsp::location::decode_location`],
/// the same decoder `goto-location!` jumps with, so path, line, and column all
/// describe one position. A malformed location aborts the whole batch. The
/// path shown is the URI's own, not [`Editor::resolve_buffer_path`]'s
/// canonical form: the row echoes what the server sent. Each `JsonHandle`
/// carries its producing server's encoding, so a batch mixing responses
/// decodes correctly.
///
/// # Column: the one sanctioned wire-unit display
/// For an open buffer the column is an exact grapheme column measured on its
/// current (possibly unsaved) text, matching where `goto-location!` lands. For
/// a target with no open buffer the file is not read (too costly for a row the
/// user may never select), so the wire `character` is shown verbatim. It
/// equals the grapheme column unless non-ASCII text precedes it on the line.
/// This is the only place HUME renders a wire unit.
///
/// Buffer lookups are cached per distinct path, so many locations in few files
/// cost one resolve and buffer-store scan per file.
pub(in crate::editor) fn location_display_parts(
    state: &EditorState,
    locs: &[hume_scripting::json::JsonHandle],
) -> Result<Vec<hume_scripting::host::LocationDisplay>, String> {
    let mut open_buffer_cache: rustc_hash::FxHashMap<std::path::PathBuf, Option<BufferId>> =
        rustc_hash::FxHashMap::default();

    locs.iter()
        .map(|handle| {
            let encoding = handle.position_encoding("lsp-locations->display-parts")?;
            let wl = hume_lsp::location::decode_location(
                handle.value(),
                "lsp-locations->display-parts",
            )?;
            let path = hume_lsp::uri::uri_to_path(&wl.uri).map_err(|e| {
                format!(
                    "lsp-locations->display-parts: cannot open {}: {e}",
                    wl.uri.as_str()
                )
            })?;
            let display_path = hume_lsp::uri::uri_to_display_string(&wl.uri).map_err(|e| {
                format!(
                    "lsp-locations->display-parts: cannot open {}: {e}",
                    wl.uri.as_str()
                )
            })?;
            let resolved = Editor::resolve_buffer_path(&path, &state.cwd);
            let open_bid = *open_buffer_cache
                .entry(resolved.clone())
                .or_insert_with(|| state.buffers.find_by_path(&resolved));

            let grapheme_col_or_wire = match open_bid {
                Some(target_bid) => {
                    let text = state.buffers.get(target_bid).text();
                    wire_pos_to_grapheme_col(text, wl.pos, encoding)
                        .map(hume_rope::column::GraphemeCol::index)
                }
                // No open buffer to measure against. See this function's
                // doc for why that means the wire unit itself, not a read.
                None => Some(wl.pos.character),
            };
            Ok(hume_scripting::host::LocationDisplay {
                path: display_path,
                line: wl.pos.line,
                grapheme_col_or_wire,
                buffer: open_bid,
            })
        })
        .collect()
}

/// Clusters → wire `{"start" "end"}`, half-open like the clusters' own
/// chars.
fn clusters_to_wire(
    text: &hume_editing::text::BufferText,
    encoding: hume_rope::position_encoding::PositionEncoding,
    range: hume_rope::cluster::ClusterRange,
) -> serde_json::Value {
    let wire_range = hume_rope::position_encoding::char_range_to_wire_range(
        text.rope(),
        range.chars(),
        encoding,
    );
    hume_lsp::position::to_json_range(wire_range)
}

/// Ready-made range params from the primary selection alone, in `t`'s own
/// pane: the shape `:lsp-code-actions` needs, since its diagnostics context
/// (`lsp/primary-selection-range` in `actions.scm`) is primary-scoped too.
pub(in crate::editor) fn primary_range_params(
    state: &EditorState,
    view: &EngineView,
    lsp: &LspState,
    t: crate::editor::commands::CommandPane,
) -> Option<serde_json::Value> {
    let id = t.bid(view);
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let text = state.buffers.get(id).text();
    let covered = crate::editor::commands::pane_view(state, view, t)
        .primary()
        .covered();
    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "range": clusters_to_wire(text, encoding, covered),
    }))
}

/// Ready-made `{"textDocument" {"uri"} "ranges" [...]}` params covering
/// every *linewise* selection in `id`'s buffer, run-length-coalesced: a run
/// of selections that touch end-to-end (the next starts where the previous
/// ends)
/// collapses into one range, since an LSP range is naturally contiguous and
/// splitting a touching run into separate ranges would buy nothing. A
/// non-linewise selection is simply skipped: the caller decides what an
/// all-linewise, all-partial, or mixed selection set means
/// (`(selections-linewise? id)` is the "all of them" read; `ranges` empty
/// here is the "none of them" read). An ambiguous selection (see
/// `SelectionView::linewise_classification`) is skipped the same
/// way, including from the touch check, so a stray cursor can't bridge two
/// real linewise neighbors into one coalesced range that silently reformats
/// the blank line between them too. `None` only when `t`'s buffer has no
/// path or no attached server, matching every other params builder in this
/// file.
pub(in crate::editor) fn linewise_ranges_params(
    state: &EditorState,
    view: &EngineView,
    lsp: &LspState,
    t: crate::editor::commands::CommandPane,
) -> Option<serde_json::Value> {
    let id = t.bid(view);
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let text = state.buffers.get(id).text();
    let selections = t.state(&state.panes.state, view).view(text);

    let linewise: Vec<_> = selections
        .iter()
        .filter(|sel| sel.linewise_classification() == Some(true))
        .map(|sel| sel.covered())
        .collect();
    let ranges: Vec<_> = linewise
        .chunk_by(|a, b| hume_rope::cluster::ClusterBound::from(b.start()) == a.end())
        .map(|run| clusters_to_wire(text, encoding, run[0].hull(run[run.len() - 1])))
        .collect();

    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "ranges": ranges,
    }))
}

/// `pane`'s visible line range, end-exclusive, clamped to a buffer of
/// `content_lines`, so the hook payload and `(viewport-range bid)` agree.
/// Never points past the buffer's last *content* line (not ropey's phantom
/// line past the structural trailing `\n`), even when the pane's viewport
/// height exceeds the buffer.
///
/// `height.max(1)` (not `height` directly): a `height == 0` pane (no visible
/// rows, e.g. one not yet laid out) still reports a one-line range rather
/// than an empty one, so callers always get at least the pane's top line
/// instead of a degenerate empty range.
pub(in crate::editor) fn pane_visible_range(
    pane: &Pane,
    content_lines: hume_rope::line::ContentLineCount,
) -> hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine> {
    let first_line = pane.viewport.top().line;
    // Terminal-row count added to a buffer-line index: under wrap one buffer
    // line can span multiple display lines (and therefore fewer terminal
    // rows than buffer lines), so this over-estimates how many buffer lines
    // are visible. Safe here: the range only needs to cover every buffer
    // line that *could* be visible, not name the true last one exactly.
    let height_rows = pane.viewport.height.max(1) as usize;
    let end_line = first_line
        .advance(height_rows)
        .min(content_lines.end_exclusive());
    hume_rope::offset::ExclusiveRange::new(first_line, end_line)
}

/// `(viewport-range pane)`: the visible line range (end-exclusive)
/// currently visible in `t`'s own pane. `t` is already resolved (see
/// `commands::CommandPane::resolve`) by the time this runs, so unlike every other
/// `id: BufferId`-taking function in this file, there is no "not shown"
/// case left to report here. The caller's own resolve raised on that
/// already.
pub(in crate::editor) fn viewport_range(
    state: &EditorState,
    view: &EngineView,
    t: crate::editor::commands::CommandPane,
) -> hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine> {
    let pane = &view.panes[t.pid()];
    let content_lines = state.buffers.get(t.bid(view)).text().content_line_count();
    pane_visible_range(pane, content_lines)
}

impl crate::editor::Editor {
    /// `:lsp-status` text: one line per registered server (language, root,
    /// lifecycle state, in-flight request count, negotiated encoding),
    /// followed by one line per attached buffer with its diagnostic counts.
    pub(in crate::editor) fn lsp_status_text(&self) -> String {
        let mut servers: Vec<(&str, &hume_lsp::client::LspClient)> = self
            .lsp
            .servers
            .values()
            .filter_map(|e| e.language.as_deref().map(|lang| (lang, &e.client)))
            .collect();
        servers.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.root().cmp(b.1.root())));

        let mut lines = Vec::new();
        if servers.is_empty() {
            lines.push("No LSP servers registered.".to_string());
        }
        for (language, client) in servers {
            lines.push(format!(
                "{language} @ {}: {}, {} in flight, encoding: {:?}",
                client.root().display(),
                client.state().name(),
                client.pending_count(),
                client.encoding(),
            ));
        }

        let mut buffer_lines: Vec<String> = self
            .state
            .buffers
            .iter()
            .filter_map(|(bid, buf)| {
                buf.lsp_server.map(|_| {
                    let (errors, warnings) = self.state.buffer_positions.diagnostics.counts(bid);
                    format!(
                        "  {}: {errors} error(s), {warnings} warning(s)",
                        buf.display_name()
                    )
                })
            })
            .collect();
        lines.append(&mut buffer_lines);

        lines.join("\n")
    }
}
