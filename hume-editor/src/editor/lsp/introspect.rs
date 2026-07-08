//! B3's introspection surface: capabilities, server status, generation, and
//! ready-made wire-position params. All read-only — no queueing, unlike
//! B2's request/notify (which must defer to the eval-result drain boundary
//! because they mutate the transport). These run through `EditorHostImpl`
//! directly since a Steel caller needs the value back inline.

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;

use super::LspState;
use super::diagnostics::DiagSeverity;
use crate::editor::EditorState;
use crate::editor::pane_state::PaneBufferState;

/// Resolves `server` — a registered language name, or `None` for "the
/// focused buffer's attached server" — to a running `ServerId`.
///
/// A bare language name is ambiguous when multiple workspace roots for that
/// language are running at once (the store is keyed by (language, root), not
/// language alone): this prefers the focused buffer's own server if it
/// matches, and otherwise errors rather than guessing. Shared by B2's
/// `lsp-request`/`lsp-notify` (via `Editor::resolve_lsp_server`) and B3's
/// `lsp-capabilities`.
pub(super) fn resolve_server(
    state: &EditorState,
    lsp: &LspState,
    focused_bid: BufferId,
    server: Option<&str>,
) -> Result<ServerId, String> {
    let focused_server = || state.buffers.get(focused_bid).lsp_server;
    match server {
        None => {
            focused_server().ok_or_else(|| "no LSP server attached to the current buffer".to_string())
        }
        Some(name) => {
            let matches: Vec<ServerId> = lsp
                .servers_by_key
                .iter()
                .filter(|((lang, _), _)| lang == name)
                .map(|(_, &sid)| sid)
                .collect();
            match matches.as_slice() {
                [] => Err(format!("no running LSP server for language '{name}'")),
                [sid] => Ok(*sid),
                _ => focused_server().filter(|sid| matches.contains(sid)).ok_or_else(|| {
                    format!(
                        "multiple '{name}' servers running — pass #f to use the \
                         current buffer's server"
                    )
                }),
            }
        }
    }
}

/// The registered language for `server_id` (the `servers_by_key` key, not
/// the display `command` string) — reverse of `resolve_server`'s named path.
pub(super) fn server_language(lsp: &LspState, server_id: ServerId) -> Option<String> {
    lsp.servers_by_key
        .iter()
        .find(|&(_, &id)| id == server_id)
        .map(|((lang, _), _)| lang.clone())
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
    lsp.capabilities_json.get(&sid).cloned()
}

/// One entry per running (language, root) server — `:lsp-status`'s data in
/// structured form.
pub(crate) fn server_status(lsp: &LspState) -> Vec<hume_scripting::LspServerStatusEntry> {
    lsp.servers_by_key
        .iter()
        .filter_map(|((language, root), &sid)| {
            let client = lsp.clients.get(&sid)?;
            Some(hume_scripting::LspServerStatusEntry {
                language: language.clone(),
                root: root.clone(),
                state: format!("{:?}", client.state),
                pending: client.pending_count(),
            })
        })
        .collect()
}

/// The registered language for the server attached to buffer `id`.
pub(crate) fn server_for_buffer(state: &EditorState, lsp: &LspState, id: BufferId) -> Option<String> {
    let sid = state.buffers.try_get(id)?.lsp_server?;
    server_language(lsp, sid)
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
    let client = lsp.clients.get(&sid)?;
    let uri = hume_lsp::uri::path_to_uri(path).ok()?;
    Some((uri.as_str().to_string(), client.encoding))
}

/// Ready-made `{"textDocument" {"uri"} "position" {"line" "character"}}`
/// params from `id`'s primary cursor head.
pub(crate) fn position_params(state: &EditorState, lsp: &LspState, id: BufferId) -> Option<serde_json::Value> {
    let (uri, encoding) = uri_and_encoding(state, lsp, id)?;
    let pbs = pane_buffer_state(state, id)?;
    let rope = state.buffers.get(id).text().rope();
    let (line, character) =
        hume_editing::position_encoding::char_to_wire(rope, pbs.selections.primary().head(), encoding);
    Some(serde_json::json!({
        "textDocument": {"uri": uri},
        "position": {"line": line, "character": character},
    }))
}

/// The negotiated encoding of `id`'s attached server, or UTF-16 (the spec
/// default) if `id` has no attached server — used by B5's `set-inlay-hints!`
/// to convert its wire positions to char offsets at set time.
pub(crate) fn encoding_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    id: BufferId,
) -> hume_editing::position_encoding::PositionEncoding {
    state
        .buffers
        .try_get(id)
        .and_then(|b| b.lsp_server)
        .and_then(|sid| lsp.clients.get(&sid))
        .map(|c| c.encoding)
        .unwrap_or(hume_editing::position_encoding::PositionEncoding::Utf16)
}

/// `(diagnostics-for-buffer bid #:severity floor #:range (start end))` —
/// decoded, filtered, capped-at-1000 (hub OQ default) hashmaps. `start`/`end`
/// are char offsets; `line`/`col` are the char-indexed start position,
/// ready for `goto-location!` shape 2 (F4).
pub(crate) fn diagnostics_for_buffer(
    state: &EditorState,
    lsp: &LspState,
    bid: BufferId,
    severity_floor: Option<&str>,
    range: Option<(usize, usize)>,
) -> Vec<serde_json::Value> {
    const CAP: usize = 1000;
    let floor = match severity_floor {
        None => DiagSeverity::Hint, // most lenient — no filtering
        Some("error") => DiagSeverity::Error,
        Some("warning") => DiagSeverity::Warning,
        Some("info") => DiagSeverity::Info,
        Some("hint") => DiagSeverity::Hint,
        Some(_) => return Vec::new(), // unknown floor name — nothing qualifies
    };
    let (start, end) = range.unwrap_or((0, usize::MAX));
    let Some(rope) = state.buffers.try_get(bid).map(|b| b.text().rope()) else {
        return Vec::new();
    };

    lsp.diagnostics
        .for_range(bid, start..end, floor)
        .take(CAP)
        .map(|d| {
            let line = rope.char_to_line(d.start.min(rope.len_chars()));
            let col = d.start.min(rope.len_chars()) - rope.line_to_char(line);
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
        .collect()
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
pub(crate) fn range_params(state: &EditorState, lsp: &LspState, id: BufferId) -> Option<serde_json::Value> {
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
    let (start_line, start_char) = hume_editing::position_encoding::char_to_wire(rope, start_c, encoding);
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
