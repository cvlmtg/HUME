//! The introspection surface: capabilities, attached servers, server status,
//! wire-position decoding and diagnostics reads. All read-only: no queueing,
//! unlike request/notify (which must defer to the eval-result drain boundary
//! because they mutate the transport). These run through `EditorHostImpl`
//! directly since a Steel caller needs the value back inline.

use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_lsp::backend::ServerId;

use super::LspState;
use super::diagnostics::DiagSeverity;
use super::features::provider;
use crate::editor::Editor;
use crate::editor::EditorState;
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
/// `feature` or `method`, or `None` when it advertises none or is not
/// running.
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

/// One entry per running server instance: `:lsp-status`'s data in
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
        })
        .collect()
}

/// Every running instance, ordered by name then root.
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
            let char_col = text.columns().char_col(line, clamped_start);
            let grapheme_col = text.columns().grapheme_col(line, clamped_start);
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
    Some(text.columns().grapheme_col(line, target.offset()))
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
/// Rows naming the same path, line and column collapse into the first, so
/// several servers' answers to one request merge. Two servers counting one
/// unopened non-ASCII position in different encodings still show twice: the
/// column of an unopened target is the wire one.
///
/// Buffer lookups are cached per distinct path, so many locations in few files
/// cost one resolve and buffer-store scan per file.
pub(in crate::editor) fn location_display_parts(
    state: &EditorState,
    locs: &[hume_scripting::json::JsonHandle],
) -> Result<Vec<hume_scripting::host::LocationDisplay>, String> {
    let mut open_buffer_cache: rustc_hash::FxHashMap<std::path::PathBuf, Option<BufferId>> =
        rustc_hash::FxHashMap::default();
    let mut seen = rustc_hash::FxHashSet::default();

    let rows: Vec<hume_scripting::host::LocationDisplay> = locs
        .iter()
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
                location: handle.clone(),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(rows
        .into_iter()
        .filter(|row| seen.insert((row.path.clone(), row.line, row.grapheme_col_or_wire)))
        .collect())
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
    /// `:lsp-status` text: one line per running server (name, languages,
    /// root, lifecycle state, in-flight request count, negotiated
    /// encoding), followed by one line per attached buffer with its
    /// servers and diagnostic counts, then the servers a stop left
    /// stopped.
    pub(in crate::editor) fn lsp_status_text(&self) -> String {
        let lsp = &self.state.lsp;
        let instances = sorted_instances(lsp);

        let mut lines = Vec::new();
        if instances.is_empty() {
            lines.push("No LSP servers running.".to_string());
        }
        for instance in instances {
            let languages = lsp.registry.languages_of(&instance.name);
            lines.push(format!(
                "{} [{}] @ {}: {}, {} in flight, encoding: {:?}",
                instance.name,
                languages.join(", "),
                instance.client.root().display(),
                instance.client.state().name(),
                instance.client.pending_count(),
                instance.client.encoding(),
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
