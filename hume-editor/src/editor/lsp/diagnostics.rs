//! Diagnostics store: `publishDiagnostics` lands here, converted to char
//! offsets at ingest, coalesced per drain batch, remapped through every
//! subsequent edit. Bulk never reaches Steel: Steel gets
//! a signal + bounded pulls.

use std::sync::Arc;

use hume_core::{BufferId, DiagSeverity, Severity};
use hume_editing::changeset::ChangeSet;
use hume_lsp::backend::ServerId;
use hume_lsp::diagnostic::{PublishedDiagnostics, WireDiagnostic};
use hume_lsp::sync::wire_version;
use hume_platform::path::AbsolutePath;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_scripting::host::DiagnosticCounts;
#[cfg(test)]
use lsp_types::PublishDiagnosticsParams;
use ropey::Rope;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::editor::{Editor, EditorState};
use hume_decorations::{Anchored, RangeAnchored, SourceStore};

#[derive(Debug, Clone)]
pub(in crate::editor) struct StoredDiag {
    pub(in crate::editor) start: CharOffset,
    pub(in crate::editor) end: CharOffset,
    pub(in crate::editor) severity: DiagSeverity,
    pub(in crate::editor) message: String,
    pub(in crate::editor) code: Option<String>,
    pub(in crate::editor) source: Option<String>,
    /// The `Diagnostic` exactly as the server sent it
    /// (`textDocument/codeAction` needs to echo this back verbatim as
    /// `context.diagnostics`: the server's quickfixes are gated on the
    /// client showing the diagnostic it's fixing, and rebuilding this from
    /// `start`/`end`'s char offsets would mean Steel fabricating wire
    /// positions itself, which the encoding-safety rule forbids). Fields
    /// `lsp_types` does not model, and `data`, survive. `Arc`-wrapped so
    /// `diagnostics-for-buffer` can hand
    /// each entry's `"raw"` to Scheme as a `JsonHandle` sharing this same
    /// allocation, instead of cloning the value to build one.
    pub(in crate::editor) raw: Arc<serde_json::Value>,
    /// The publishing server, and its negotiated encoding at ingest time.
    /// Together they tag `raw`'s `JsonHandle`
    /// (`diagnostics::diagnostics_for_buffer`) so a wire position inside it
    /// decodes correctly even after that server has restarted or detached.
    pub(in crate::editor) server: hume_scripting::ServerRef,
    pub(in crate::editor) encoding: hume_rope::position_encoding::PositionEncoding,
}

impl Anchored for StoredDiag {
    type Anchor = CharOffset;
    fn anchor(&self) -> CharOffset {
        self.start
    }
}

impl RangeAnchored for StoredDiag {
    fn end(&self) -> CharOffset {
        self.end
    }
    fn set_range(&mut self, range: ExclusiveRange<CharOffset>) {
        self.start = range.start;
        self.end = range.end;
    }
}

/// Wraps the same generic `SourceStore<K, T>` the decoration kinds share
/// (`hume-decorations`'s `decorations.rs`), keyed by `ServerId` instead of a plugin-chosen
/// source name. `set`/`remap_ranges`/`remove_buffer` are the shared
/// write/remap machinery; the diagnostics-specific reads (`for_range`'s
/// severity/range filter, `counts`) stay here since no decoration kind
/// needs them.
#[derive(Default)]
pub(in crate::editor) struct DiagnosticsStore {
    store: SourceStore<ServerId, StoredDiag>,
    /// Each buffer and floor a `diagnostics-for-buffer` read found past
    /// [`DIAGNOSTICS_READ_CAP`], until a whole-buffer read at that floor
    /// finds it under.
    over_cap: FxHashSet<(BufferId, DiagSeverity)>,
}

impl DiagnosticsStore {
    /// Replaces one server's diagnostics for `bid` (already coalesced:
    /// the caller keeps only the last `publishDiagnostics` per (server,
    /// uri) within a drain batch). `SourceStore::set` sorts by `start`
    /// (`StoredDiag`'s `Anchored` impl), so callers need not pre-sort
    /// themselves.
    pub(in crate::editor) fn replace(
        &mut self,
        server: ServerId,
        bid: BufferId,
        diags: Vec<StoredDiag>,
    ) {
        self.store.set(server, bid, diags);
    }

    /// Remaps every stored range for `bid` through `cs`: `PositionStores`
    /// calls this for every change to the buffer's text. A range collapsed
    /// to empty by a covering deletion is dropped, not kept as a zero-width
    /// entry (`SourceStore::remap_ranges`' shared policy, the same one
    /// `ExtraHighlightEntry` uses).
    pub(in crate::editor) fn remap_through(&mut self, bid: BufferId, cs: &ChangeSet) {
        self.store.remap_ranges(bid, cs);
    }

    /// Drops what `server` published for `bid`, when `server` detaches from
    /// it or stops being asked for its diagnostics. `false` when it had
    /// published nothing for `bid`.
    pub(in crate::editor::lsp) fn remove_source_for_buffer(
        &mut self,
        server: ServerId,
        bid: BufferId,
    ) -> bool {
        self.store.remove_source_for_buffer(&server, bid)
    }

    /// Drops every diagnostic for `bid`, across every server. Called when
    /// the buffer is closed (a pure memory-leak fix there: `BufferId` is a
    /// versioned slotmap key, so a future slot reuse can never alias with
    /// the closed buffer's stale entry) and on `:e!` reload (where it *is*
    /// a correctness fix: offsets computed against the pre-reload text
    /// must not survive against the new content). Returns whether anything
    /// was actually removed, so a reload caller only fires
    /// `OnDiagnosticsChanged` when the display actually changes.
    pub(in crate::editor) fn remove_buffer(&mut self, bid: BufferId) -> bool {
        self.over_cap.retain(|(over, _)| *over != bid);
        self.store.remove_buffer(bid)
    }

    /// Every buffer with at least one stored diagnostic, from any server,
    /// including one whose server has since crashed: a crash leaves its
    /// buffers attached and their entries in place, so `:reload-config`'s
    /// resync can still replay `OnDiagnosticsChanged` for them. A stopped
    /// server's entries go when its buffers detach.
    pub(in crate::editor) fn buffers_with_diagnostics(
        &self,
    ) -> impl Iterator<Item = BufferId> + '_ {
        self.store.buffers()
    }

    /// Notes that a read of `bid` at `floor` matched `matched` diagnostics,
    /// over the whole buffer when `whole`. `true` for the first read past
    /// [`DIAGNOSTICS_READ_CAP`] since a whole-buffer read at `floor` found
    /// `bid` under it. A ranged read under the cap says nothing about the
    /// rest of the buffer.
    fn note_read(
        &mut self,
        bid: BufferId,
        floor: DiagSeverity,
        matched: usize,
        whole: bool,
    ) -> bool {
        if matched > DIAGNOSTICS_READ_CAP {
            self.over_cap.insert((bid, floor))
        } else {
            if whole {
                self.over_cap.remove(&(bid, floor));
            }
            false
        }
    }

    /// Counts `bid`'s diagnostics by severity; a severity below `floor`
    /// counts as none.
    pub(in crate::editor) fn counts(&self, bid: BufferId, floor: DiagSeverity) -> DiagnosticCounts {
        let mut counts = DiagnosticCounts::default();
        for (_server, d) in self.store.for_buffer(bid) {
            if d.severity <= floor {
                *counts.of(d.severity) += 1;
            }
        }
        counts
    }

    /// Each server's own entries are sorted by `start` (`SourceStore::set`),
    /// but with 2+ servers publishing for the same buffer, concatenating
    /// them in server order would not be globally sorted. Callers that
    /// assume start-ascending order (e.g. `lsp/goto-next-diagnostic`'s
    /// nearest-match logic) would jump to whichever server happened to be
    /// iterated first rather than the nearest diagnostic. Collected and
    /// sorted once here so every caller gets a globally ordered result
    /// without re-deriving it.
    pub(in crate::editor::lsp) fn for_range(
        &self,
        bid: BufferId,
        range: ExclusiveRange<CharOffset>,
        floor: DiagSeverity,
    ) -> impl ExactSizeIterator<Item = &StoredDiag> {
        let mut out: Vec<&StoredDiag> = self.for_range_unsorted(bid, range, floor).collect();
        out.sort_by_key(|d| d.start);
        out.into_iter()
    }

    /// Every stored diagnostic's `(start, end)` for `bid`, in order.
    #[cfg(test)]
    pub(in crate::editor) fn spans_for_test(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.for_range(
            bid,
            ExclusiveRange::new(CharOffset::new(0), CharOffset::new(usize::MAX)),
            DiagSeverity::Hint,
        )
        .map(|d| (d.start.index(), d.end.index()))
    }

    /// [`Self::for_range`] without the cross-server ordering pass, for a
    /// caller whose own result doesn't depend on the order it sees these in
    /// (the sign bridge folds them into a per-line winner; the underline
    /// bridge re-sorts what it builds). Lazy, so it never collects at all.
    pub(in crate::editor) fn for_range_unsorted(
        &self,
        bid: BufferId,
        range: ExclusiveRange<CharOffset>,
        floor: DiagSeverity,
    ) -> impl Iterator<Item = &StoredDiag> {
        let (lo, hi) = (range.start, range.end);
        self.store
            .groups_for_buffer(bid)
            .flat_map(move |(_server, diags)| {
                // Each server's slice is sorted by `start`
                // (`SourceStore::set`), so everything past the first
                // `start >= hi` can't overlap `range`. `end` isn't sorted,
                // so the lower bound still needs a full scan from the front.
                let upper = diags.partition_point(|d| d.start < hi);
                diags[..upper].iter()
            })
            .filter(move |d| d.severity <= floor && d.end > lo)
    }
}

fn map_severity(sev: Option<lsp_types::DiagnosticSeverity>) -> DiagSeverity {
    match sev {
        // Spec: absent severity is left to the client to interpret. Error
        // keeps it maximally visible rather than silently downgrading it.
        None | Some(lsp_types::DiagnosticSeverity::ERROR) => DiagSeverity::Error,
        Some(lsp_types::DiagnosticSeverity::WARNING) => DiagSeverity::Warning,
        Some(lsp_types::DiagnosticSeverity::HINT) => DiagSeverity::Hint,
        // INFORMATION and any future/unknown severity value.
        Some(_) => DiagSeverity::Info,
    }
}

/// Widens a zero-length `[pos, pos)` range to one char: HUME diagnostic
/// decorations, like selections, are never empty. Widens forward by
/// default; widens backward instead when `pos` is at end-of-line or
/// end-of-buffer, so the range never crosses into the next line. Always
/// succeeds: the buffer invariant (`len_chars() >= 1`, always ending in a
/// structural `\n`) guarantees at least the newline itself to widen onto,
/// even on the minimal `"\n"` buffer, matching how a selection can cover
/// that same newline cell.
fn widen_zero_length(rope: &Rope, pos: CharOffset) -> ExclusiveRange<CharOffset> {
    let len = rope.len_chars();
    if pos.index() < len && rope.char(pos.index()) != '\n' {
        ExclusiveRange::new(pos, pos.shift(1))
    } else if pos.index() > 0 {
        ExclusiveRange::new(pos.shift(-1), pos)
    } else {
        ExclusiveRange::new(CharOffset::new(0), CharOffset::new(1))
    }
}

/// How many `canonicalize()` calls one `drain_lsp` may spend resolving the
/// files servers publish for. A call costs about 15 µs with a warm cache
/// (measured in a release build), so this bounds the resolution to about a
/// millisecond; the publishes beyond it wait for the next drain. A publish
/// naming an open buffer's own path spends none.
pub(in crate::editor) const CANONICALIZE_PER_DRAIN: usize = 64;

/// What ingesting one publish came to.
pub(in crate::editor) enum Ingest {
    /// Ingested, or dropped; the buffer that took diagnostics, if any.
    Done(Option<BufferId>),
    /// The drain's `canonicalize()` budget was spent before this publish
    /// could be resolved to a file.
    Deferred(PublishedDiagnostics),
}

/// Publishes waiting to be ingested, the newest per `(server, uri)`: what
/// one drain gathers, and what a drain whose budget ran out leaves for the
/// next.
#[derive(Default)]
pub(in crate::editor) struct PublishQueue {
    // clippy's `mutable_key_type` flags `lsp_types::Uri` for the `Cell`s
    // inside its underlying `fluent_uri::Uri`'s parse-offset cache, but
    // `Uri`'s `Hash`/`PartialEq`/`Eq` are hand-implemented against
    // `.as_str()` only (lsp-types 0.97.0's uri.rs), which those cells
    // never affect. A false positive for this specific type.
    #[allow(clippy::mutable_key_type)]
    queued: FxHashMap<(ServerId, lsp_types::Uri), PublishedDiagnostics>,
}

#[allow(clippy::mutable_key_type)]
impl PublishQueue {
    /// Queues `published`, replacing an older publish for the same server
    /// and URI.
    pub(in crate::editor) fn offer(&mut self, server: ServerId, published: PublishedDiagnostics) {
        self.queued
            .insert((server, published.uri.clone()), published);
    }

    /// Puts back a publish that could not be ingested yet, unless a newer
    /// one for the same server and URI has been offered since.
    pub(in crate::editor) fn defer(&mut self, server: ServerId, published: PublishedDiagnostics) {
        self.queued
            .entry((server, published.uri.clone()))
            .or_insert(published);
    }

    pub(in crate::editor) fn take_all(&mut self) -> Vec<(ServerId, PublishedDiagnostics)> {
        self.queued
            .drain()
            .map(|((server, _), published)| (server, published))
            .collect()
    }

    /// Drops what `server` published: it is gone, and nothing it sent
    /// should cost a path lookup.
    pub(in crate::editor) fn forget_server(&mut self, server: ServerId) {
        self.queued.retain(|(sid, _), _| *sid != server);
    }

    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    #[cfg(test)]
    pub(in crate::editor) fn len(&self) -> usize {
        self.queued.len()
    }
}

/// `params` as a classified publish: each diagnostic's wire value is its
/// serialization.
#[cfg(test)]
pub(in crate::editor) fn published_for_test(
    params: PublishDiagnosticsParams,
) -> PublishedDiagnostics {
    PublishedDiagnostics {
        uri: params.uri,
        version: params.version,
        diagnostics: params
            .diagnostics
            .into_iter()
            .map(|diagnostic| WireDiagnostic {
                raw: serde_json::to_value(&diagnostic).expect("a diagnostic serializes"),
                diagnostic,
            })
            .collect(),
        skipped: Vec::new(),
    }
}

impl Editor {
    /// [`Self::ingest_publish_diagnostics`] for a publish built as typed
    /// params.
    #[cfg(test)]
    pub(in crate::editor) fn ingest_typed_publish_for_test(
        &mut self,
        server_id: ServerId,
        params: PublishDiagnosticsParams,
    ) -> Option<BufferId> {
        self.ingest_now(server_id, published_for_test(params))
    }

    /// Ingests one already-coalesced, already-classified `publishDiagnostics`
    /// payload (the caller kept only the last one per (server, uri) within
    /// this drain batch; a payload `hume-lsp` cannot read at all never
    /// reaches here, it is classified as a `ServerNotification` instead, and
    /// a diagnostic that does not parse was left out of it). Drops silently
    /// (one Trace line) when the URI doesn't
    /// resolve to an open buffer: v1 never opens a buffer just to hold
    /// diagnostics. Gives back the buffer actually ingested into, so the
    /// caller can fire `OnDiagnosticsChanged` once per touched buffer.
    ///
    /// A URI path that is already an open buffer's own path names that
    /// buffer, with no filesystem call. Any other path is resolved the way a
    /// typed path keys its buffer ([`AbsolutePath::resolve`]), so a file not
    /// on disk yet is found through a symlinked directory too.
    /// `canonicalize_budget` meters those resolutions across a drain: each
    /// costs one unit, and the publish is deferred when none is left.
    pub(in crate::editor) fn ingest_publish_diagnostics(
        &mut self,
        server_id: ServerId,
        published: PublishedDiagnostics,
        canonicalize_budget: &mut usize,
    ) -> Ingest {
        let Some(path) = hume_lsp::uri::uri_to_path(&published.uri)
            .ok()
            .and_then(|path| AbsolutePath::from_path(&path, &self.state.cwd).ok())
        else {
            self.report(
                Severity::Trace,
                "lsp: publishDiagnostics with an unresolvable URI".to_string(),
            );
            return Ingest::Done(None);
        };
        if let Some(bid) = self.state.buffers.find_by_path(&path) {
            return Ingest::Done(self.ingest_into(server_id, published, bid));
        }
        if *canonicalize_budget == 0 {
            return Ingest::Deferred(published);
        }
        *canonicalize_budget -= 1;
        let resolved = match path.resolve() {
            Ok(resolved) => resolved,
            Err(e) => {
                self.report(
                    Severity::Trace,
                    format!(
                        "lsp: publishDiagnostics for an unresolvable path: {}: {e}",
                        path.display()
                    ),
                );
                return Ingest::Done(None);
            }
        };
        let Some(bid) = self.state.buffers.find_by_path(&resolved) else {
            self.report(
                Severity::Trace,
                format!(
                    "lsp: publishDiagnostics for an unopened buffer: {}",
                    resolved.display()
                ),
            );
            return Ingest::Done(None);
        };
        Ingest::Done(self.ingest_into(server_id, published, bid))
    }

    /// [`Self::ingest_publish_diagnostics`] with no budget, for a test's
    /// publish ingested at once.
    #[cfg(test)]
    fn ingest_now(
        &mut self,
        server_id: ServerId,
        published: PublishedDiagnostics,
    ) -> Option<BufferId> {
        let mut unlimited = usize::MAX;
        match self.ingest_publish_diagnostics(server_id, published, &mut unlimited) {
            Ingest::Done(bid) => bid,
            Ingest::Deferred(_) => unreachable!("an unlimited budget never defers"),
        }
    }

    /// The part of an ingest that follows finding the buffer: the publish
    /// checked against the buffer's version and stored.
    fn ingest_into(
        &mut self,
        server_id: ServerId,
        published: PublishedDiagnostics,
        bid: BufferId,
    ) -> Option<BufferId> {
        // A publish computed against an older version would convert its
        // positions against text that has since moved on. The server has
        // already received our newer didChange(s) and will republish
        // against the current version shortly. Drop it rather than store
        // positions that are quietly wrong until then; the existing
        // (already-remapped) stored diagnostics keep displaying meanwhile.
        // Absent version is always ingested (older/simpler servers omit it).
        if let Some(v) = published.version
            && v != wire_version(self.state.buffers.get(bid).text().generation())
        {
            self.report(
                Severity::Trace,
                format!("lsp: dropping publishDiagnostics for a stale version ({v})"),
            );
            return None;
        }
        self.state
            .store_server_diagnostics(server_id, bid, published.diagnostics)
            .then_some(bid)
    }
}

impl EditorState {
    /// Stores `diagnostics` as everything `server_id` reports for `bid`,
    /// converted to char offsets against the buffer as it is now. `false`
    /// when they are dropped: only a server attached to the buffer, and
    /// asked for its diagnostics there, has them shown, and a server
    /// untracked since it reported has no negotiated encoding to decode
    /// them against.
    pub(in crate::editor) fn store_server_diagnostics(
        &mut self,
        server_id: ServerId,
        bid: BufferId,
        diagnostics: Vec<WireDiagnostic>,
    ) -> bool {
        let admitted =
            self.buffer_positions
                .lsp
                .admits(bid, server_id, hume_core::LspFeature::Diagnostics);
        if !admitted {
            let name = self.lsp_server_name(server_id);
            self.report(
                Severity::Trace,
                format!("lsp: dropping diagnostics from '{name}', which is not reporting diagnostics for this buffer"),
            );
            return false;
        }
        let Some((server, encoding)) = self.lsp.instances.origin(server_id) else {
            self.report(
                Severity::Trace,
                "lsp: dropping diagnostics from an untracked server".to_string(),
            );
            return false;
        };
        let rope = self.buffers.get(bid).text().rope().clone();

        let stored: Vec<StoredDiag> = diagnostics
            .into_iter()
            .map(|WireDiagnostic { diagnostic: d, raw }| {
                let raw = Arc::new(raw);
                let range = hume_lsp::position::lsp_range_to_chars(&rope, &d.range, encoding);
                let range = if range.start == range.end {
                    widen_zero_length(&rope, range.start)
                } else {
                    range
                };
                StoredDiag {
                    start: range.start,
                    end: range.end,
                    severity: map_severity(d.severity),
                    message: d.message,
                    code: d.code.map(|c| match c {
                        lsp_types::NumberOrString::Number(n) => n.to_string(),
                        lsp_types::NumberOrString::String(s) => s,
                    }),
                    source: d.source,
                    raw,
                    server: server.clone(),
                    encoding,
                }
            })
            .collect();

        self.buffer_positions
            .diagnostics
            .replace(server_id, bid, stored);
        true
    }

    /// Reports the diagnostics of a report that did not parse: a Warning the
    /// first time for `server_id`, a Trace line after, so a server that
    /// reports on every keystroke does not fill the log.
    pub(in crate::editor) fn note_skipped_diagnostics(
        &mut self,
        server_id: ServerId,
        skipped: &[String],
    ) {
        let Some(first) = skipped.first() else {
            return;
        };
        let severity = if self.lsp.instances.first_skipped_report(server_id) {
            Severity::Warning
        } else {
            Severity::Trace
        };
        let name = self.lsp_server_name(server_id);
        self.report(
            severity,
            format!(
                "lsp: '{name}' sent {} diagnostic(s) that do not parse and were skipped; \
                 the rest are shown (first error: {first})",
                skipped.len()
            ),
        );
    }
}

/// The floor a diagnostics read applies: `requested`, or with none
/// `lsp-diagnostics-min-severity`, the floor `update_highlight_providers`
/// draws underlines at, so a caller (the statusline, the diagnostics
/// plugin's EOL summary and gutter signs) agrees with what's on screen
/// unless it asks for a different cut.
pub(in crate::editor) fn severity_floor(
    state: &EditorState,
    requested: Option<DiagSeverity>,
) -> DiagSeverity {
    requested.unwrap_or(state.settings.lsp_diagnostics_min_severity)
}

/// How many diagnostics `diagnostics-for-buffer` returns at most: the first
/// ones by position. Converting each to a Steel hash costs about 0.75 µs,
/// and core:lsp pulls a buffer's diagnostics on every publish.
/// [`diagnostics_for_buffer`] traces a read it cuts, once until a
/// whole-buffer read at that floor finds the buffer under it again.
pub(in crate::editor) const DIAGNOSTICS_READ_CAP: usize = 1000;

/// `(diagnostics-for-buffer bid #:severity floor #:range (start-char . end-char))`:
/// decoded, filtered hashmaps, at most [`DIAGNOSTICS_READ_CAP`] of them.
/// `start-char`/`end-char` are char offsets, *addressing* units that
/// `goto-location!`'s `'char-offset` takes as they are. `start-char` is clamped onto the buffer's last
/// character, so a diagnostic past the end starts on the final line break.
/// `start-line` is
/// the line of `start-char` and `grapheme-col`
/// its column, for *display*: the one unit every HUME surface (statusline,
/// diagnostics, LSP location lists) shows the user; never render a char
/// offset directly. `end-line` is one past the last line the diagnostic touches,
/// clamped the same way `start-line` is, so `[start-line, end-line)` is
/// never empty. The diagnostics plugin's gutter-sign pass marks every line
/// in it. `severity-rank` is [`DiagSeverity::rank`] alongside the
/// `severity` symbol, so a caller compares severities by it rather than
/// re-deriving the order from the symbol. A `#:range` must lie within the
/// buffer's text:
/// it may end at the text's end, but not past it, and may not end before it
/// starts.
///
/// With no `#:severity`, the floor is [`severity_floor`]'s default.
pub(in crate::editor) fn diagnostics_for_buffer(
    state: &mut EditorState,
    bid: BufferId,
    floor: Option<DiagSeverity>,
    range: Option<(usize, usize)>,
) -> Result<Vec<hume_scripting::host::DiagnosticEntry>, String> {
    let floor = severity_floor(state, floor);
    let whole = range.is_none();
    let Some(text) = state.buffers.try_get(bid).map(|b| b.text()) else {
        return Ok(Vec::new());
    };
    // `text.end()`, not a `usize::MAX` sentinel: the buffer's own exclusive
    // end bound is the honest "whole buffer" default, and only becomes
    // available once `text` is in hand, as does the rope `#:range` is
    // checked against.
    let range = match range {
        None => ExclusiveRange::new(CharOffset::new(0), text.end()),
        Some((start, end)) => crate::editor::buffer::offsets::checked_char_range(text, start, end)
            .map_err(|e| format!("#:range {e}"))?,
    };

    let matched = state
        .buffer_positions
        .diagnostics
        .for_range(bid, range, floor);
    let count = matched.len();
    let entries = matched
        .take(DIAGNOSTICS_READ_CAP)
        .map(|d| {
            // Clamped to the last char, not `len_chars()`: a server can
            // report a diagnostic anchored at end-of-file, and `len_chars()`
            // itself resolves to the buffer's trailing phantom empty line
            // (every buffer ends with a structural `\n`; see
            // `host_impl/decorations.rs`'s `content_line`).
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
                severity: d.severity,
                message: d.message.clone(),
                code: d.code.clone(),
                source: d.source.clone(),
                raw: std::sync::Arc::clone(&d.raw),
                server: d.server.clone(),
                encoding: d.encoding,
            }
        })
        .collect();
    if state
        .buffer_positions
        .diagnostics
        .note_read(bid, floor, count, whole)
    {
        let name = state.buffers.get(bid).display_name();
        state.report(
            Severity::Trace,
            format!(
                "lsp: {name}: diagnostics-for-buffer matched {count} diagnostics at {} or above \
                 and returns the first {DIAGNOSTICS_READ_CAP} by position",
                floor.name()
            ),
        );
    }
    Ok(entries)
}

#[cfg(test)]
mod tests;
