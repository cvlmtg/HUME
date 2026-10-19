//! The introspection surface: capabilities, attached servers, server status,
//! wire-position decoding and diagnostics reads. All read-only: no queueing,
//! unlike request/notify (which must defer to the eval-result drain boundary
//! because they mutate the transport). These run through `EditorHostImpl`
//! directly since a Steel caller needs the value back inline.

use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_lsp::backend::ServerId;

use super::LspState;
use super::diagnostics::DiagSeverity;
use super::features::provider;
use crate::editor::Editor;
use crate::editor::EditorState;
use crate::editor::settings::SettingText;
use hume_scripting::{CapabilityQuery, ListEntry, LspFeature, ServerName};

/// The feature a standard request `method` belongs to.
pub(in crate::editor) fn method_feature(method: &str) -> Option<LspFeature> {
    super::features::requirement(method).map(|r| r.feature)
}

/// What `server` advertises for `query`, borrowed from its capabilities.
pub(in crate::editor) fn provider_of<'a>(
    lsp: &'a LspState,
    server: ServerId,
    query: CapabilityQuery<'_>,
) -> Option<&'a serde_json::Value> {
    let caps = lsp.instances.get(server)?.client.capabilities_json()?;
    provider(caps, query)
}

/// What `(lsp-capability …)` hands to Steel: `server`'s capability for
/// `feature` or `method`, or `None` when it advertises none, has not
/// finished its handshake, or was stopped. A crashed instance keeps its last
/// capabilities.
pub(in crate::editor) fn capability(
    lsp: &LspState,
    server: ServerId,
    query: CapabilityQuery<'_>,
) -> Option<std::sync::Arc<serde_json::Value>> {
    provider_of(lsp, server, query).map(|value| std::sync::Arc::new(value.clone()))
}

/// `server`'s raw wire capabilities. See `LspClient::capabilities_json`'s
/// doc comment for why this, not the typed decode, is what
/// `(lsp-capabilities …)` hands to Steel. `Arc`-wrapped: this clone is a
/// refcount bump, not a deep copy of the capabilities blob.
pub(in crate::editor) fn capabilities(
    lsp: &LspState,
    server: ServerId,
) -> Option<std::sync::Arc<serde_json::Value>> {
    lsp.instances
        .get(server)?
        .client
        .capabilities_json()
        .cloned()
}

/// `bid`'s attached servers: every one, in any state, with neither
/// `feature` nor `method`; otherwise the ones a request of that feature or
/// method reaches now, none when the route reaches none.
pub(in crate::editor) fn servers(
    state: &EditorState,
    bid: BufferId,
    feature: Option<hume_scripting::LspFeature>,
    method: Option<&str>,
) -> Result<Vec<hume_scripting::ServerRef>, String> {
    if state.buffers.try_get(bid).is_none() {
        return Err(format!("invalid buffer id {bid:?}"));
    }
    if feature.is_none() && method.is_none() {
        return Ok(state
            .buffer_positions
            .lsp
            .servers(bid)
            .filter_map(|sid| state.lsp.instances.server_ref(sid))
            .collect());
    }
    let spec = hume_scripting::RouteSpec { feature, to: None };
    Ok(super::route::route(state, bid, method, &spec)
        .map(|routed| routed.into_iter().map(|r| r.server).collect())
        .unwrap_or_default())
}

/// One entry per server instance: `:lsp-status`'s data in
/// structured form.
pub(in crate::editor) fn server_status(
    lsp: &LspState,
) -> Vec<hume_scripting::LspServerStatusEntry> {
    sorted_instances(lsp)
        .into_iter()
        .map(|i| hume_scripting::LspServerStatusEntry {
            name: i.name.clone(),
            languages: lsp
                .registry
                .languages_of(&i.name)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            root: i.client.root().to_path_buf(),
            state: i.client.state().name(),
            pending: i.client.pending_count(),
            encoding: i.client.encoding(),
        })
        .collect()
}

/// Every instance, ordered by name then root.
fn sorted_instances(lsp: &LspState) -> Vec<&super::instances::Instance> {
    let mut instances: Vec<_> = lsp.instances.iter().map(|(_, i)| i).collect();
    instances.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.client.root().cmp(b.client.root()))
    });
    instances
}

/// Whether `name` is registered.
pub(in crate::editor) fn server_registered(lsp: &LspState, name: &ServerName) -> bool {
    lsp.registry.contains(name)
}

/// The servers `language`'s buffers attach to, in order, with their filters.
pub(in crate::editor) fn language_servers(lsp: &LspState, language: &str) -> Vec<ListEntry> {
    lsp.registry
        .plan(language)
        .into_iter()
        .map(|p| ListEntry {
            name: p.name.clone(),
            filter: p.filter,
        })
        .collect()
}

/// A buffer's attached servers' loading state. Drives the statusline's
/// loading spinner (`statusline::elements::diagnostics`).
pub(crate) enum LspActivity {
    /// No attached server, or none starting or reporting progress: nothing
    /// to animate.
    Idle,
    /// Some attached server is mid `initialize` handshake.
    Starting,
    /// A `$/progress` task (indexing, loading, ...) is in flight: the most
    /// recently begun one, if the server is running more than one. Carries
    /// no title: the statusline only shows the spinner + percentage
    /// (`statusline::elements::diagnostics`); the underlying task's
    /// title is reachable via `LspState::progress_title_for_test` for tests
    /// that need to assert the begin/report merge machine.
    Progress { percentage: Option<u32> },
}

/// `id`'s attached servers' current [`LspActivity`]: `Starting` while any
/// is starting, otherwise the progress of the first, in attachment order,
/// that reports any.
pub(crate) fn activity(state: &EditorState, id: BufferId) -> LspActivity {
    let instances = || {
        state
            .buffer_positions
            .lsp
            .servers(id)
            .filter_map(|sid| state.lsp.instances.get(sid))
    };
    if instances().any(|i| i.client.state() == hume_lsp::client::ServerState::Starting) {
        return LspActivity::Starting;
    }
    instances()
        .find_map(|i| i.progress.last())
        .map_or(LspActivity::Idle, |(_, task)| LspActivity::Progress {
            percentage: task.percentage,
        })
}

/// The negotiated encoding of a specific running server: the counterpart
/// for a caller that already has a `ServerId` in hand instead of resolving
/// one from a `BufferId` (a server-initiated request, `publishDiagnostics`,
/// or a workspace edit a server sent). `None` for a server that is not
/// tracked.
pub(in crate::editor) fn server_encoding(
    lsp: &LspState,
    sid: ServerId,
) -> Option<hume_rope::position_encoding::PositionEncoding> {
    lsp.instances.get(sid).map(|i| i.client.encoding())
}

/// Wire `[start, end)` range → char range, decoded in `encoding`, for
/// `lsp-range->char-range`. `None` if `id` is unknown or the range reaches a
/// line past the document end (see
/// [`hume_rope::position_encoding::checked_wire_range_to_char_range`]).
/// `encoding` is the caller's own resolve: the tagged producing-server
/// encoding of the response the range was read out of
/// (`JsonHandle::position_encoding`), never `id`'s *currently* attached
/// server: the two can diverge (a restart renegotiates a different encoding,
/// a detach leaves none), and the tag is always the encoding the range was
/// actually written in. No `LspState` lookup needed at all, for the same
/// reasoning as [`label_slice`]'s own `encoding` parameter, below.
///
/// A range's `end` legitimately lands exactly at the buffer's `len_chars()`
/// (`set-extra-highlights!`'s `validate_range` accepts that boundary), so
/// the document end `(line count, 0)` is accepted. Point-anchored callers
/// want the opposite; see [`wire_point_to_char_for_buffer`].
pub(in crate::editor) fn wire_range_to_chars_for_buffer(
    state: &EditorState,
    id: BufferId,
    range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> Option<hume_rope::offset::ExclusiveRange<hume_rope::offset::CharOffset>> {
    let rope = state.buffers.try_get(id)?.text().rope();
    hume_rope::position_encoding::checked_wire_range_to_char_range(rope, range, encoding)
}

/// Wire `(line, character)` → char offset, for `lsp-position->char-offset`.
/// `None` if `id` is unknown, or when the position is not on a content line
/// (the buffer's trailing phantom line, every buffer ending with a structural
/// `\n`, or past it). The offset a clamp would give there is `len_chars()`,
/// and a point-anchored decoration setter (`set-inlay-hints!`'s
/// `validate_offset`) rejects it outright: handing it back here would let a single
/// stale server response (a request that raced an edit) fail the caller's
/// *entire* hint batch (`collect::<Result<Vec<_>, _>>` in `host_impl.rs`)
/// instead of just being filtered out, one entry, by the caller's own `#f`
/// check. `encoding` is the caller's own resolve, as for
/// [`wire_range_to_chars_for_buffer`].
pub(in crate::editor) fn wire_point_to_char_for_buffer(
    state: &EditorState,
    id: BufferId,
    pos: hume_rope::position_encoding::WirePos,
    encoding: hume_rope::position_encoding::PositionEncoding,
) -> Option<hume_rope::offset::CharOffset> {
    let rope = state.buffers.try_get(id)?.text().rope();
    hume_rope::position_encoding::checked_wire_to_char(rope, pos, encoding)
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

/// `(diagnostics-for-buffer bid #:severity floor #:range (start-char . end-char))`:
/// decoded, filtered, capped-at-1000 hashmaps. `start-char`/`end-char`
/// are char offsets, *addressing* units that `goto-location!`'s `'char-offset`
/// takes as they are. `start-char` is clamped onto the buffer's last
/// character, so a diagnostic past the end starts on the final line break.
/// `start-line` is
/// the line of `start-char` and `grapheme-col`
/// its column, for *display*: the one unit every HUME surface (statusline,
/// diagnostics, LSP location lists) shows the user; never render a char
/// offset directly. `end-line` is one past the last line the diagnostic touches,
/// clamped the same way `start-line` is, so `[start-line, end-line)` is
/// never empty. The diagnostics plugin's gutter-sign pass marks every line
/// in it. `severity-rank` is `DiagSeverity`'s own `Ord` discriminant (`0`
/// for error, counting up to `3` for hint) alongside the `severity` string,
/// so a caller compares severities by this rather than re-deriving the same
/// order from the string. Errors loudly on an unknown `#:severity` name
/// (e.g. `'warn` typoed for `'warning`) rather than silently returning
/// nothing that qualifies. A `#:range` must lie within the buffer's text:
/// it may end at the text's end, but not past it, and may not end before it
/// starts.
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
    range: Option<(usize, usize)>,
) -> Result<Vec<hume_scripting::host::DiagnosticEntry>, String> {
    use hume_rope::offset::{CharOffset, ExclusiveRange};
    const CAP: usize = 1000;
    let floor = match severity_floor.map(DiagSeverity::parse) {
        None => state.settings.lsp_diagnostics_severity_floor,
        Some(Ok(f)) => f,
        Some(Err(e)) => return Err(format!("invalid severity floor: {e}")),
    };
    let Some(text) = state.buffers.try_get(bid).map(|b| b.text()) else {
        return Ok(Vec::new());
    };
    // `text.end()`, not a `usize::MAX` sentinel: the buffer's own exclusive
    // end bound is the honest "whole buffer" default, and only becomes
    // available once `text` is in hand, as does the rope `#:range` is
    // checked against.
    let range = match range {
        None => ExclusiveRange::new(CharOffset::new(0), text.end()),
        Some((start, end)) => {
            let checked = |idx, which| {
                crate::editor::host_impl::checked_char_offset(text, idx)
                    .map_err(|e| format!("#:range {which}: {e}"))
            };
            let start = checked(start, "start")?;
            let end = checked(end, "end")?;
            if end < start {
                return Err(format!(
                    "#:range {}..{} ends before it starts",
                    start.index(),
                    end.index()
                ));
            }
            ExclusiveRange::new(start, end)
        }
    };

    let entries = state
        .buffer_positions
        .diagnostics
        .for_range(bid, range, floor)
        .take(CAP)
        .map(|d| {
            // Clamped to the last char, not `len_chars()`: a server can
            // report a diagnostic anchored at end-of-file, and `len_chars()`
            // itself resolves to the buffer's trailing phantom empty line
            // (every buffer ends with a structural `\n`; see
            // `hume-editor/src/editor/host_impl.rs`'s `line_start_offset`).
            // Landing there instead of the last content line would hand
            // plugins a `line` that later fails the fail-fast bound check
            // every decoration setter enforces. The final `\n` is a valid
            // cluster start, so a diagnostic on it keeps its offset.
            let last_char = text.last_char();
            let clamped_start = d.start.min(last_char);
            let line = text.char_to_line(clamped_start);
            let grapheme_col = text.columns().grapheme_col(line, clamped_start);
            // `end-line` mirrors `line`'s clamp so a range that reaches (or
            // overshoots) end-of-file still names the buffer's last content
            // line rather than the phantom trailing one. An empty range, or
            // one ending at a line's start, takes `line` as its last line.
            let last_line = text
                .char_to_line(d.end.retreat_saturating(1).min(last_char))
                .max(line);
            let end_line = last_line.advance(1);
            hume_scripting::host::DiagnosticEntry {
                start_char: clamped_start.index(),
                end_char: d.end.index(),
                start_line: line.index(),
                end_line: end_line.index(),
                grapheme_col: grapheme_col.index(),
                severity: d.severity.to_text(),
                severity_rank: d.severity as u8,
                message: d.message.clone(),
                code: d.code.clone(),
                source: d.source.clone(),
                raw: std::sync::Arc::clone(&d.raw),
                server: d.server.clone(),
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
pub(crate) fn spinner_frame(state: &EditorState) -> usize {
    state.lsp.spinner.frame
}

/// Filesystem path, wire line, and column for a batch of raw
/// `Location`/`LocationLink` LSP locations, backing
/// `lsp-locations->places` and `lsp-locations->merged-places` (goto/references
/// drawer rows). The errors do not name a builtin; the builtin adds its own.
///
/// Each location is decoded once through [`hume_lsp::location::decode_location`],
/// the same decoder `goto-location!` jumps with, so path, line, and column all
/// describe one position. A malformed location aborts the whole batch. The
/// path shown is the URI's own, not [`Editor::resolve_buffer_path`]'s
/// canonical form: the row echoes what the server sent. Each handle comes
/// paired with its producing server's encoding, so a batch mixing responses
/// decodes correctly.
///
/// A location with no place `goto-location!` can jump to is
/// [`LocationPlace::Skipped`](hume_scripting::host::LocationPlace::Skipped):
/// a URI that is not a `file://` one, or a line past the content of the
/// file's open buffer, found by the same `wire_to_cluster` the jump uses. A
/// file that is not open is not read, so its stale line still shows, and
/// the jump answers `#f`.
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
pub(in crate::editor) fn location_places(
    state: &EditorState,
    locs: &[(
        hume_scripting::json::JsonHandle,
        hume_rope::position_encoding::PositionEncoding,
    )],
) -> Result<Vec<hume_scripting::host::LocationPlace>, String> {
    use hume_scripting::host::{LocationPlace, Place};

    let mut open_buffer_cache: rustc_hash::FxHashMap<std::path::PathBuf, Option<BufferId>> =
        rustc_hash::FxHashMap::default();
    let mut rows = Vec::with_capacity(locs.len());

    for (handle, encoding) in locs {
        let wl = hume_lsp::location::decode_location(handle.value())?;
        let cannot_open =
            |e: hume_lsp::uri::UriError| format!("cannot open {}: {e}", wl.uri.as_str());
        let path = match hume_lsp::uri::uri_to_path(&wl.uri) {
            Ok(path) => path,
            Err(hume_lsp::uri::UriError::NotFileScheme) => {
                rows.push(LocationPlace::Skipped);
                continue;
            }
            Err(e) => return Err(cannot_open(e)),
        };
        let display_path = hume_lsp::uri::uri_to_display_string(&wl.uri).map_err(cannot_open)?;
        let open_bid = *open_buffer_cache.entry(path).or_insert_with_key(|path| {
            let resolved = Editor::resolve_buffer_path(path, &state.cwd);
            state.buffers.find_by_path(&resolved)
        });

        let grapheme_col_or_wire = match open_bid {
            Some(target_bid) => {
                let text = state.buffers.get(target_bid).text();
                let Some((line, target)) = super::wire_to_cluster(text, wl.pos, *encoding) else {
                    rows.push(LocationPlace::Skipped);
                    continue;
                };
                text.columns().grapheme_col(line, target.offset()).index()
            }
            // No open buffer to measure against. See this function's
            // doc for why that means the wire unit itself, not a read.
            None => wl.pos.character,
        };
        rows.push(LocationPlace::Found(Place {
            path: display_path,
            line: wl.pos.line,
            grapheme_col_or_wire,
            buffer: open_bid,
            location: handle.clone(),
        }));
    }
    Ok(rows)
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

/// Pane `pid`'s visible line range (end-exclusive) between frames, for
/// `(viewport-range pane)` and the `OnViewportChange` payload. Lays out
/// against the current bands first, since a band can open or close
/// between frames.
pub(in crate::editor) fn viewport_range(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
) -> hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine> {
    state.relayout(view);
    let pane = &view.panes[pid];
    let content_lines = state
        .buffers
        .get(pane.buffer_id())
        .text()
        .content_line_count();
    pane_visible_range(pane, content_lines)
}

impl crate::editor::Editor {
    /// `:lsp-status` text: one line per running server (name, languages,
    /// root, lifecycle state, in-flight request count, negotiated
    /// encoding), followed by one line per attached buffer with its
    /// servers and diagnostic counts, then the servers a stop left
    /// stopped.
    pub(in crate::editor) fn lsp_status_text(&self) -> String {
        let lsp = &self.state.lsp;
        let status = server_status(lsp);

        let mut lines = Vec::new();
        if status.is_empty() {
            lines.push("No LSP servers running.".to_string());
        }
        for entry in status {
            lines.push(format!(
                "{} [{}] @ {}: {}, {} in flight, encoding: {:?}",
                entry.name,
                entry.languages.join(", "),
                entry.root.display(),
                entry.state,
                entry.pending,
                entry.encoding,
            ));
        }

        for (bid, buf) in self.state.buffers.iter() {
            let names: Vec<String> = self
                .state
                .buffer_positions
                .lsp
                .servers(bid)
                .filter_map(|sid| lsp.instances.get(sid).map(|i| i.name.to_string()))
                .collect();
            if names.is_empty() {
                continue;
            }
            let (errors, warnings) = self.state.buffer_positions.diagnostics.counts(bid);
            lines.push(format!(
                "  {} [{}]: {errors} error(s), {warnings} warning(s)",
                buf.display_name(),
                names.join(", ")
            ));
        }

        let mut stopped: Vec<String> = lsp
            .stopped
            .iter()
            .map(|(name, root)| format!("  {name} @ {}", root.display()))
            .collect();
        if !stopped.is_empty() {
            stopped.sort_unstable();
            lines.push("Stopped (:lsp-restart starts them again):".to_string());
            lines.extend(stopped);
        }

        lines.join("\n")
    }
}
