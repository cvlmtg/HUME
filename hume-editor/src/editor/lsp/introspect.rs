//! The introspection surface: capabilities, server status, generation, and
//! ready-made wire-position params. All read-only — no queueing, unlike
//! request/notify (which must defer to the eval-result drain boundary
//! because they mutate the transport). These run through `EditorHostImpl`
//! directly since a Steel caller needs the value back inline.

use std::ops::Range;

use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_lsp::backend::ServerId;

use super::LspState;
use super::diagnostics::DiagSeverity;
use super::registry::LanguageName;
use crate::editor::Editor;
use crate::editor::EditorState;

/// Resolves `server` — a registered language name, or `None` for "the
/// focused buffer's attached server" — to a running `ServerId`.
///
/// A bare language name is ambiguous when multiple workspace roots for that
/// language are running at once (the store is keyed by (language, root), not
/// language alone): this prefers the focused buffer's own server if it
/// matches, and otherwise errors rather than guessing. Shared by
/// `lsp-request`/`lsp-notify` (via `Editor::resolve_lsp_server`) and
/// `lsp-capabilities`.
///
/// Errors loudly on a Crashed (or otherwise untracked) server rather than
/// resolving to it — its sends are silently dropped (`send_or_queue`), so a
/// caller would otherwise learn of the problem only as a generic timeout at
/// the request's deadline. `Starting` still resolves: `send_or_queue`'s
/// Starting-queue correctly defers the send until the handshake completes.
pub(super) fn resolve_server(
    state: &EditorState,
    lsp: &LspState,
    focused_bid: BufferId,
    server: Option<&str>,
) -> Result<ServerId, String> {
    let focused_server = || state.buffers.get(focused_bid).lsp_server;
    let sid = match server {
        None => focused_server()
            .ok_or_else(|| "no LSP server attached to the current buffer".to_string())?,
        Some(name) => {
            let matches: Vec<ServerId> = lsp
                .servers
                .iter()
                .filter(|(_, e)| e.language.as_deref() == Some(name))
                .map(|(&sid, _)| sid)
                .collect();
            match matches.as_slice() {
                [] => return Err(format!("no running LSP server for language '{name}'")),
                [sid] => *sid,
                _ => focused_server()
                    .filter(|sid| matches.contains(sid))
                    .ok_or_else(|| {
                        format!(
                            "multiple '{name}' servers running — pass #f to use the \
                         current buffer's server"
                        )
                    })?,
            }
        }
    };
    match lsp.servers.get(&sid).map(|e| e.client.state()) {
        Some(hume_lsp::client::ServerState::Starting | hume_lsp::client::ServerState::Running) => {
            Ok(sid) // send_or_queue handles Starting's deferred send correctly
        }
        Some(hume_lsp::client::ServerState::Crashed) => {
            Err("lsp server crashed — run :lsp-restart".to_string())
        }
        Some(hume_lsp::client::ServerState::Dead) | None => Err("lsp server stopped".to_string()),
    }
}

/// The registered language for `server_id` — reverse of `resolve_server`'s
/// named path.
pub(super) fn server_language(lsp: &LspState, server_id: ServerId) -> Option<LanguageName> {
    lsp.servers.get(&server_id)?.language.clone()
}

/// The server's raw wire capabilities — see `LspClient::capabilities_json`'s
/// doc comment for why this, not the typed decode, is what
/// `(lsp-capabilities …)` must hand to Steel.
pub(crate) fn capabilities(
    state: &EditorState,
    lsp: &LspState,
    focused_bid: BufferId,
    server: Option<&str>,
) -> Option<serde_json::Value> {
    let sid = resolve_server(state, lsp, focused_bid, server).ok()?;
    lsp.servers.get(&sid)?.client.capabilities_json().cloned()
}

/// One entry per running (language, root) server — `:lsp-status`'s data in
/// structured form.
pub(crate) fn server_status(lsp: &LspState) -> Vec<hume_scripting::LspServerStatusEntry> {
    lsp.servers
        .values()
        .filter_map(|e| {
            let language = e.language.clone()?;
            Some(hume_scripting::LspServerStatusEntry {
                language,
                root: e.client.root().to_path_buf(),
                state: format!("{:?}", e.client.state()),
                pending: e.client.pending_count(),
            })
        })
        .collect()
}

/// The registered language for the server attached to buffer `id`.
pub(crate) fn server_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> Option<LanguageName> {
    let sid = state.buffers.try_get(id)?.lsp_server?;
    server_language(lsp, sid)
}

/// A buffer's attached server's loading state — drives the statusline's
/// loading spinner (`ui::statusline::elements::diagnostics`).
pub(crate) enum LspActivity {
    /// No attached server, a `Running` server with no progress task in
    /// flight, or a `Crashed`/`Dead` one — nothing to animate.
    Idle,
    /// Mid `initialize` handshake.
    Starting,
    /// A `$/progress` task (indexing, loading, ...) is in flight — the most
    /// recently begun one, if the server is running more than one. Carries
    /// no title: the statusline only shows the spinner + percentage
    /// (`ui::statusline::elements::diagnostics`); the underlying task's
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

/// Whether `language` currently has a `register-lsp-server!` config —
/// registered, not necessarily attached/running. Distinguishes "no server
/// registered" from "registered but still starting", which
/// `server_for_buffer` (attachment, not registration) can't tell apart.
pub(crate) fn registered_for_language(lsp: &LspState, language: &str) -> bool {
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
/// params from `id`'s primary cursor head, in the pane currently showing it.
pub(crate) fn position_params(
    state: &EditorState,
    view: &EngineView,
    lsp: &LspState,
    id: BufferId,
) -> Option<serde_json::Value> {
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let pbs = state.shown_buffer_state(view, id)?;
    let rope = state.buffers.get(id).text().rope();
    let (line, character) =
        hume_rope::position_encoding::char_to_wire(rope, pbs.selections.primary().head(), encoding);
    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "position": {"line": line, "character": character},
    }))
}

/// The negotiated encoding of `id`'s attached server, or `None` if `id` is
/// unknown or has no attached (tracked) server — the fallible core shared by
/// [`encoding_for_buffer`] (clamps to UTF-16) and [`wire_to_char_for_buffer`]
/// (refuses instead of guessing).
fn negotiated_encoding(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> Option<hume_rope::position_encoding::PositionEncoding> {
    let sid = state.buffers.try_get(id)?.lsp_server?;
    Some(lsp.servers.get(&sid)?.client.encoding())
}

/// The negotiated encoding of `id`'s attached server, or UTF-16 (the spec
/// default) if `id` has no attached server — used by `set-inlay-hints!`
/// to convert its wire positions to char offsets at set time.
pub(crate) fn encoding_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> hume_rope::position_encoding::PositionEncoding {
    negotiated_encoding(state, lsp, id)
        .unwrap_or(hume_rope::position_encoding::PositionEncoding::Utf16)
}

/// Wire `(line, character)` → char offset in `id`'s attached server's
/// negotiated encoding, for `lsp-range->offsets`. `None` if `id` is unknown
/// or has no attached server — unlike `set-inlay-hints!`'s
/// `encoding_for_buffer`, this refuses rather than guessing UTF-16: the
/// caller has no way to supply an encoding, so a wrong silent answer (only
/// visible on non-ASCII lines) is worse than a visible `#f`.
///
/// Clamps rather than errors on an out-of-range `line`/`character`, same as
/// `wire_to_char` itself — a range's `end` legitimately lands exactly at
/// the buffer's `len_chars()` (`set-extra-highlights!`'s `validate_range`
/// accepts that boundary), so this must not reject it. Point-anchored
/// callers want the opposite; see [`wire_point_to_char_for_buffer`].
pub(crate) fn wire_to_char_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
    line: usize,
    character: usize,
) -> Option<usize> {
    let rope = state.buffers.try_get(id)?.text().rope();
    let encoding = negotiated_encoding(state, lsp, id)?;
    Some(hume_rope::position_encoding::wire_to_char(
        rope, line, character, encoding,
    ))
}

/// Wire `(line, character)` → char offset, for `lsp-position->offset`.
/// Same conversion as [`wire_to_char_for_buffer`], but refuses (`None`)
/// when the result lands at `len_chars()` — the position `wire_to_char`
/// clamps a past-end `line` onto (the buffer's trailing phantom line, every
/// buffer ending with a structural `\n`). A point-anchored decoration
/// setter (`set-inlay-hints!`'s `validate_offset`) rejects that offset
/// outright: handing it back here would let a single stale server response
/// (a request that raced an edit) fail the caller's *entire* hint batch
/// (`collect::<Result<Vec<_>, _>>` in `host_impl.rs`) instead of just being
/// filtered out, one entry, by the caller's own `#f` check.
pub(crate) fn wire_point_to_char_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
    line: usize,
    character: usize,
) -> Option<usize> {
    let offset = wire_to_char_for_buffer(state, lsp, id, line, character)?;
    let len_chars = state.buffers.try_get(id)?.text().rope().len_chars();
    (offset < len_chars).then_some(offset)
}

/// `label` sliced by a `ParameterInformation.label` `[start, end)` wire
/// offset pair, for `lsp-label-offsets->text`.
///
/// The one place a wire offset indexes a *server-authored string* rather
/// than a document: the offsets address `SignatureInformation.label`, which
/// never reaches a buffer, so none of the `bid`-anchored converters above
/// fit. `id` is here only to name the server whose negotiated encoding the
/// offsets are counted in — the same negotiation their `Position` siblings
/// ride, since a server converts every outgoing offset through one layer.
///
/// `None` when `id` has no attached server, refusing rather than guessing
/// UTF-16 for the same reason [`wire_to_char_for_buffer`] does: the wrong
/// answer is invisible until the label holds a non-ASCII character.
pub(crate) fn label_slice_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
    label: &str,
    start: usize,
    end: usize,
) -> Option<String> {
    let encoding = negotiated_encoding(state, lsp, id)?;
    let range =
        hume_rope::position_encoding::wire_offsets_to_byte_range(label, start, end, encoding);
    Some(label[range].to_string())
}

/// `(diagnostics-for-buffer bid #:severity floor #:range (start . end))` —
/// decoded, filtered, capped-at-1000 hashmaps. `start`/`end`
/// are char offsets; `line`/`char-col` are the char-indexed start position,
/// ready for `goto-location!` shape 2 — an *addressing* unit, exact and
/// lossless. `grapheme-col` is the same position as a grapheme column
/// instead, for *display* — the one unit every HUME surface (statusline,
/// diagnostics, LSP location lists) shows the user; never render `char-col`
/// directly. `end-line` is the range's *end* clamped and converted the same
/// way `line` is — the diagnostics plugin's gutter-sign pass expands
/// `[line, end-line]` inclusive to mark every line a multi-line diagnostic
/// touches. `severity-rank` is `DiagSeverity`'s own `Ord` discriminant (`0`
/// for error, counting up to `3` for hint) alongside the `severity` string,
/// so a caller compares severities by this rather than re-deriving the same
/// order from the string. Errors loudly on an unknown `#:severity` name
/// (e.g. `'warn` typoed for `'warning`) rather than silently returning
/// nothing that qualifies.
///
/// With no `#:severity`, defaults to `lsp.diagnostics-severity-floor` — the
/// same floor `update_highlight_providers` applies to underlines, so a
/// caller (e.g. the diagnostics plugin's EOL summary and gutter signs)
/// agrees with what's on screen unless it explicitly asks for a different
/// cut.
pub(crate) fn diagnostics_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    bid: BufferId,
    severity_floor: Option<&str>,
    range: Option<(usize, usize)>,
) -> Result<Vec<serde_json::Value>, String> {
    const CAP: usize = 1000;
    let floor = match severity_floor.map(str::parse::<DiagSeverity>) {
        None => state.settings.lsp_diagnostics_severity_floor,
        Some(Ok(f)) => f,
        Some(Err(e)) => return Err(e),
    };
    let (start, end) = range.unwrap_or((0, usize::MAX));
    let Some(text) = state.buffers.try_get(bid).map(|b| b.text()) else {
        return Ok(Vec::new());
    };

    let entries = lsp
        .diagnostics
        .for_range(bid, start..end, floor)
        .take(CAP)
        .map(|d| {
            // Clamped to the last *content* char, not `len_chars()`: a
            // server can report a diagnostic anchored at end-of-file, one
            // past the buffer's last real char, and `len_chars()` itself
            // resolves to the buffer's trailing phantom empty line (every
            // buffer ends with a structural `\n` — see
            // `hume-editor/src/editor/host_impl.rs`'s `line_start_offset`).
            // Landing there instead of the buffer's last content line would
            // hand plugins a `line` that later fails the fail-fast bound
            // check every decoration setter now enforces.
            let last_content_char = text.len_chars().saturating_sub(1);
            let clamped_start = d.start.min(last_content_char);
            let line = text.char_to_line(clamped_start);
            let char_col = hume_editing::lines::char_col_in_line(text, line, clamped_start);
            let grapheme_col =
                hume_editing::grapheme::grapheme_col_in_line(text, line, clamped_start);
            // `end-line` mirrors `line`'s clamp so a range that reaches (or
            // overshoots) end-of-file still names the buffer's last content
            // line rather than the phantom trailing one — the gutter-sign
            // plugin expands `[line, end-line]` inclusive to mark every line
            // a multi-line diagnostic touches.
            let end_line = text.char_to_line(d.end.saturating_sub(1).min(last_content_char));
            serde_json::json!({
                "start": d.start,
                "end": d.end,
                "line": line,
                "end-line": end_line,
                "char-col": char_col,
                "grapheme-col": grapheme_col,
                "severity": d.severity.to_string(),
                // `DiagSeverity`'s own `Ord` discriminant (0 = error … 3 =
                // hint, lower is more severe) — the single encoding of
                // severity order, so Scheme compares by this instead of
                // re-deriving the same ranking from the `severity` string.
                "severity-rank": d.severity as u8,
                "message": d.message,
                "code": d.code,
                "source": d.source,
                "raw": d.raw,
            })
        })
        .collect();
    Ok(entries)
}

/// `(diagnostic-counts bid)` → `(errors, warnings)`.
pub(crate) fn diagnostic_counts(lsp: &LspState, bid: BufferId) -> (usize, usize) {
    lsp.diagnostics.counts(bid)
}

/// `line`/`character` clamped into `text`'s addressable range and converted
/// to a grapheme column — `None` when `line` names no real content, rather
/// than silently reporting a column under a `line` that doesn't match it.
///
/// The bound is the last *content* line, so a server's past-end response and
/// the buffer's own phantom trailing line (the one the structural `\n`
/// creates) both return `None`. Clamping to the ropey domain instead would
/// admit the phantom line and report column 1 of a line that has no
/// characters — a drawer row pointing one line past the file's end.
fn wire_pos_to_grapheme_col(
    text: &hume_editing::text::BufferText,
    line: usize,
    character: usize,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> Option<usize> {
    if line > text.last_content_line() {
        return None;
    }
    let char_pos =
        hume_rope::position_encoding::wire_to_char(text.rope(), line, character, encoding);
    Some(hume_editing::grapheme::grapheme_col_in_line(
        text, line, char_pos,
    ))
}

/// Filesystem path, wire line, and column for a batch of raw
/// `Location`/`LocationLink` LSP locations — the display companion to
/// `wire_to_char_for_buffer`'s address conversion, backing
/// `lsp-locations->display-parts`. `lsp/location-display` calls this once
/// per drawer build so a goto/references row has something to show in the
/// column slot.
///
/// Each location is decoded once, through [`hume_lsp::location::decode_location`]
/// — the same decoder `goto-location!` uses for the jump — so the path, the
/// displayed line, and the column all come from that one decode. A drawer
/// row that read `range.start.line` a second time in Scheme, or decoded the
/// URI a second time to render the path, could end up naming a position it
/// didn't measure; see that function's doc for why a malformed location
/// aborts the whole batch.
/// The path is the URI's own, not [`Editor::resolve_buffer_path`]'s
/// canonicalisation of it: resolving symlinks is right for *finding* the
/// file, but a drawer row should echo the path the server actually sent.
///
/// Every location shares `focused_bid`'s attached server's negotiated
/// encoding — that server produced every one of these responses, whichever
/// file each location points into (same rationale `GotoTarget::Wire` uses
/// for the actual jump, `edits.rs`'s `resolve_goto_target`).
///
/// # The one sanctioned exception to "never render a wire unit"
/// An **open** buffer's rope is used as-is (its unsaved text, if modified —
/// the column reported is the one the user will land on after
/// `goto-location!` jumps there, and that jump already carries the same
/// staleness against a server response that may predate the edit), giving
/// an exact grapheme column. For a target with **no open buffer** this
/// function does not read the file — reading a whole file from disk just to
/// refine one column for a row the user may never select is out of
/// proportion to the value — so it reports the location's own wire
/// `character` verbatim instead. That number is an offset in the server's
/// negotiated encoding (a byte offset under `utf-8`, UTF-16 code units
/// otherwise), not a grapheme count: on a line that is
/// ASCII up to the target position — nearly all code — the two coincide
/// exactly; they diverge only when non-ASCII text sits earlier on the same
/// line, and then by more than one (a 3-byte CJK character counts 3, a ZWJ
/// emoji family counts roughly 25). This is the *only* place in HUME a wire
/// unit is rendered directly — everywhere else the "never render `char_col`
/// or a wire position" rule holds without exception. A future refinement is
/// to render an unmeasured column visually distinctly (e.g. italic) once the
/// drawer can style parts of a row, rather than showing it identically to a
/// measured one.
///
/// Resolving a URI's path against the buffer store is the only work left
/// once a target isn't read — cached per distinct path (not per location)
/// so a batch with many locations in few files pays one `canonicalize` +
/// buffer-store scan per file, not one per location.
pub(crate) fn location_display_parts(
    state: &EditorState,
    lsp: &LspState,
    focused_bid: BufferId,
    locs: &[serde_json::Value],
) -> Result<Vec<hume_scripting::host::LocationDisplay>, String> {
    let encoding = encoding_for_buffer(state, lsp, focused_bid);
    let mut open_buffer_cache: rustc_hash::FxHashMap<std::path::PathBuf, Option<BufferId>> =
        rustc_hash::FxHashMap::default();

    locs.iter()
        .map(|loc| {
            let wl = hume_lsp::location::decode_location(loc, "lsp-locations->display-parts")?;
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
                Some(bid) => {
                    let text = state.buffers.get(bid).text();
                    wire_pos_to_grapheme_col(text, wl.line, wl.character, encoding)
                }
                // No open buffer to measure against — see this function's
                // doc for why that means the wire unit itself, not a read.
                None => Some(wl.character),
            };
            Ok(hume_scripting::host::LocationDisplay {
                path: display_path,
                line: wl.line,
                grapheme_col_or_wire,
            })
        })
        .collect()
}

/// Char range → wire `{"start" "end"}`. HUME selections are inclusive
/// (`end_c` names the last included char); LSP ranges are half-open, so
/// `end` is one grapheme cluster past — `next_grapheme_boundary`, not a raw
/// `+ 1`, since `end_c` may be the first char of a multi-char cluster (`é` =
/// e + U+0301, a ZWJ emoji sequence): stepping by one raw char would land
/// the wire range mid-cluster.
fn char_range_to_wire(
    text: &hume_editing::text::BufferText,
    encoding: hume_rope::position_encoding::PositionEncoding,
    start_c: usize,
    end_c: usize,
) -> serde_json::Value {
    let end_exclusive = hume_editing::grapheme::next_grapheme_boundary(text, end_c);
    let ((start_line, start_char), (end_line, end_char)) =
        hume_rope::position_encoding::char_range_to_wire_range(
            text.rope(),
            start_c,
            end_exclusive,
            encoding,
        );
    serde_json::json!({
        "start": {"line": start_line, "character": start_char},
        "end": {"line": end_line, "character": end_char},
    })
}

/// Ready-made range params from `id`'s primary selection alone — the shape
/// `:lsp-code-actions` needs, since its diagnostics context
/// (`lsp/primary-selection-range` in `actions.scm`) is primary-scoped too.
pub(crate) fn primary_range_params(
    state: &EditorState,
    view: &EngineView,
    lsp: &LspState,
    id: BufferId,
) -> Option<serde_json::Value> {
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let sel = state.shown_buffer_state(view, id)?.selections.primary();
    let text = state.buffers.get(id).text();
    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "range": char_range_to_wire(text, encoding, sel.start(), sel.end()),
    }))
}

/// Ready-made `{"textDocument" {"uri"} "ranges" [...]}` params covering
/// every *linewise* selection in `id`'s buffer, run-length-coalesced: a run
/// of selections that touch end-to-end (`next.start() == prev.end() + 1`)
/// collapses into one range, since an LSP range is naturally contiguous and
/// splitting a touching run into separate ranges would buy nothing. A
/// non-linewise selection is simply skipped — the caller decides what an
/// all-linewise, all-partial, or mixed selection set means
/// (`(selections-linewise? id)` is the "all of them" read; `ranges` empty
/// here is the "none of them" read). `None` only when `id` has no path, no
/// attached server, or isn't shown in any pane, matching every other params
/// builder in this file.
pub(crate) fn linewise_ranges_params(
    state: &EditorState,
    view: &EngineView,
    lsp: &LspState,
    id: BufferId,
) -> Option<serde_json::Value> {
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let text = state.buffers.get(id).text();
    let selections = &state.shown_buffer_state(view, id)?.selections;

    let linewise: Vec<_> = selections
        .iter_sorted()
        .filter(|sel| hume_editing::selection::is_selection_linewise(text, sel))
        .collect();
    let ranges: Vec<_> = linewise
        .chunk_by(|a, b| b.start() == a.end() + 1)
        .map(|run| char_range_to_wire(text, encoding, run[0].start(), run[run.len() - 1].end()))
        .collect();

    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "ranges": ranges,
    }))
}

/// `pane`'s visible line range, end-exclusive, clamped to a buffer of
/// `content_lines` — the single computation shared by `queue_viewport_change`
/// (pane -> its own range, for the `on-viewport-change` hook payload) and
/// [`viewport_range`] (buffer -> the pane showing it, for the synchronous
/// `(viewport-range bid)` builtin, which wraps this range in a dotted-pair
/// wire value). Clamped to `content_lines` so the range never points past the
/// buffer's last *content* line — not ropey's phantom line past the
/// structural trailing `\n` — even when the pane's viewport height exceeds
/// the buffer.
///
/// `height.max(1)` (not `height` directly): a `height == 0` pane (no visible
/// rows, e.g. one not yet laid out) still reports a one-line range rather
/// than an empty one, so callers always get at least the pane's top line
/// instead of a degenerate empty range.
pub(crate) fn pane_visible_range(pane: &Pane, content_lines: usize) -> Range<usize> {
    let first_line = pane.viewport.top_line;
    let visible_rows = pane.viewport.height.max(1) as usize;
    let end_line = (first_line + visible_rows).min(content_lines);
    first_line..end_line
}

/// `(viewport-range bid)` — the visible line range (end-exclusive) currently
/// visible for `id`, or `None` if `id` isn't shown in any pane (a background
/// or hidden buffer). With the same buffer open in two panes, the focused
/// pane's range wins — no less arbitrary than any other tie-break, since a
/// per-buffer decoration store (inlay hints) can only hold one range per
/// buffer regardless of how many panes show it.
pub(crate) fn viewport_range(
    state: &EditorState,
    view: &EngineView,
    id: BufferId,
) -> Option<Range<usize>> {
    let pane_id = state.pane_showing_buffer(view, id)?;
    let pane = view.panes.get(pane_id)?;
    let content_lines = state.buffers.try_get(id)?.text().content_line_count();
    Some(pane_visible_range(pane, content_lines))
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
                "{language} @ {} — {:?}, {} in flight, encoding: {:?}",
                client.root().display(),
                client.state(),
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
                    let (errors, warnings) = self.lsp.diagnostics.counts(bid);
                    format!(
                        "  {} — {errors} error(s), {warnings} warning(s)",
                        buf.display_name()
                    )
                })
            })
            .collect();
        lines.append(&mut buffer_lines);

        lines.join("\n")
    }
}
