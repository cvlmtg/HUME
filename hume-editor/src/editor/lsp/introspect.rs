//! The introspection surface: capabilities, server status, generation, and
//! ready-made wire-position params. All read-only — no queueing, unlike
//! request/notify (which must defer to the eval-result drain boundary
//! because they mutate the transport). These run through `EditorHostImpl`
//! directly since a Steel caller needs the value back inline.

use std::ops::Range;

use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_lsp::backend::ServerId;

use super::LspState;
use super::diagnostics::DiagSeverity;
use super::registry::LanguageName;
use crate::editor::EditorState;
use crate::editor::pane_state::PaneBufferState;

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

/// Decoded `ServerCapabilities`, cached at handshake completion
/// (`Editor::dispatch_lsp_action`'s `BecameRunning` arm) rather than
/// reconverted on every call.
pub(crate) fn capabilities(
    state: &EditorState,
    lsp: &LspState,
    focused_bid: BufferId,
    server: Option<&str>,
) -> Option<serde_json::Value> {
    let sid = resolve_server(state, lsp, focused_bid, server).ok()?;
    lsp.servers.get(&sid)?.capabilities_json.clone()
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

/// The seeded `PaneBufferState` for `(state.focused_pane_id, id)` if seeded
/// there, else the first pane (any) that has `id` seeded — a buffer can be
/// open in a non-focused pane, or in no pane at all (background buffer).
fn pane_buffer_state(state: &EditorState, id: BufferId) -> Option<&PaneBufferState> {
    if let Some(pbs) = state
        .panes
        .state
        .get(state.focused_pane_id)
        .and_then(|m| m.get(id))
    {
        return Some(pbs);
    }
    state.panes.state.values().find_map(|m| m.get(id))
}

/// Shared setup for both params builders: the buffer's URI and its attached
/// server's negotiated encoding. `None` if `id` has no path or no attached
/// (tracked) server.
fn uri_and_encoding<'a>(
    state: &'a EditorState,
    lsp: &'a LspState,
    id: BufferId,
) -> Option<(String, hume_editing::position_encoding::PositionEncoding)> {
    let buf = state.buffers.try_get(id)?;
    let path = buf.path()?;
    let sid = buf.lsp_server?;
    let entry = lsp.servers.get(&sid)?;
    let uri = hume_lsp::uri::path_to_uri(path).ok()?;
    Some((uri.as_str().to_string(), entry.client.encoding()))
}

/// Ready-made `{"textDocument" {"uri"} "position" {"line" "character"}}`
/// params from `id`'s primary cursor head.
pub(crate) fn position_params(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> Option<serde_json::Value> {
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let pbs = pane_buffer_state(state, id)?;
    let rope = state.buffers.get(id).text().rope();
    let (line, character) = hume_editing::position_encoding::char_to_wire(
        rope,
        pbs.selections.primary().head(),
        encoding,
    );
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
) -> Option<hume_editing::position_encoding::PositionEncoding> {
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
) -> hume_editing::position_encoding::PositionEncoding {
    negotiated_encoding(state, lsp, id)
        .unwrap_or(hume_editing::position_encoding::PositionEncoding::Utf16)
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
    Some(hume_editing::position_encoding::wire_to_char(
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

/// `(diagnostics-for-buffer bid #:severity floor #:range (start . end))` —
/// decoded, filtered, capped-at-1000 hashmaps. `start`/`end`
/// are char offsets; `line`/`col` are the char-indexed start position,
/// ready for `goto-location!` shape 2. Errors loudly on an unknown
/// `#:severity` name (e.g. `'warn` typoed for `'warning`) rather than
/// silently returning nothing that qualifies.
///
/// With no `#:severity`, defaults to `lsp.diagnostics-severity-floor` — the
/// same floor `update_highlight_providers`/`update_sign_providers` apply to
/// underlines/gutter signs, so a caller (e.g. the diagnostics plugin's EOL
/// summary) agrees with what's on screen unless it explicitly asks for a
/// different cut.
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
    let Some(rope) = state.buffers.try_get(bid).map(|b| b.text().rope()) else {
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
            let last_content_char = rope.len_chars().saturating_sub(1);
            let line = rope.char_to_line(d.start.min(last_content_char));
            let col = d.start.min(last_content_char) - rope.line_to_char(line);
            serde_json::json!({
                "start": d.start,
                "end": d.end,
                "line": line,
                "col": col,
                "severity": d.severity.to_string(),
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

/// Ready-made `{"textDocument" {"uri"} "range" {"start" "end"}}` params from
/// `id`'s primary selection. HUME selections are inclusive (`head` names the
/// last included char); LSP ranges are half-open, so `end` is one grapheme
/// cluster past — `next_grapheme_boundary`, not a raw `+ 1`, since `end_c`
/// may be the first char of a multi-char cluster (`é` = e + U+0301, a ZWJ
/// emoji sequence): stepping by one raw char would land the wire range
/// mid-cluster.
pub(crate) fn range_params(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> Option<serde_json::Value> {
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let pbs = pane_buffer_state(state, id)?;
    let sel = pbs.selections.primary();
    let (start_c, end_c) = if sel.anchor() <= sel.head() {
        (sel.anchor(), sel.head())
    } else {
        (sel.head(), sel.anchor())
    };
    let text = state.buffers.get(id).text();
    let end_exclusive = hume_editing::grapheme::next_grapheme_boundary(text, end_c);
    let rope = text.rope();
    let (start_line, start_char) =
        hume_editing::position_encoding::char_to_wire(rope, start_c, encoding);
    let (end_line, end_char) =
        hume_editing::position_encoding::char_to_wire(rope, end_exclusive, encoding);
    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "range": {
            "start": {"line": start_line, "character": start_char},
            "end": {"line": end_line, "character": end_char},
        },
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

/// The pane currently showing buffer `id`: the focused pane if it shows `id`,
/// else the first pane (by `SlotMap` iteration order) that does, else `None`
/// if `id` isn't open in any pane. Mirrors [`pane_buffer_state`]'s
/// focused-first/any-fallback policy, but resolves against `EngineView`'s
/// live pane geometry rather than the seeded per-(pane,buffer) cursor state.
fn pane_showing_buffer(state: &EditorState, view: &EngineView, id: BufferId) -> Option<PaneId> {
    if view
        .panes
        .get(state.focused_pane_id)
        .is_some_and(|p| p.buffer_id == id)
    {
        return Some(state.focused_pane_id);
    }
    view.panes
        .iter()
        .find(|(_, p)| p.buffer_id == id)
        .map(|(pid, _)| pid)
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
    let pane_id = pane_showing_buffer(state, view, id)?;
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
