//! Diagnostics store: `publishDiagnostics` lands here, converted to char
//! offsets at ingest, coalesced per drain batch, remapped through every
//! subsequent edit. Bulk never reaches Steel — Steel gets
//! a signal + bounded pulls.

use std::collections::HashMap;
use std::ops::Range;

use hume_editing::changeset::ChangeSet;
use hume_editing::wire_to_char;
use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::sync::wire_version;
use lsp_types::PublishDiagnosticsParams;
use ropey::Rope;

use crate::editor::Editor;
use crate::editor::message_log::Severity;

/// Ordered least-to-most-lenient so `severity <= floor` means "at least as
/// severe as floor" — e.g. `floor = Warning` keeps `Error` and `Warning`,
/// drops `Info`/`Hint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl DiagSeverity {
    /// The wire-format strings `FromStr` accepts — the single source
    /// `:set global lsp.diagnostics-severity-floor=<Tab>` completion mirrors,
    /// so the two can never drift out of sync (same convention as `TabStyle`).
    pub const VALUES: &'static [&'static str] = &["error", "warning", "info", "hint"];
}

impl std::fmt::Display for DiagSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
        })
    }
}

impl std::str::FromStr for DiagSeverity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Ok(Self::Error),
            "warning" => Ok(Self::Warning),
            "info" => Ok(Self::Info),
            "hint" => Ok(Self::Hint),
            _ => Err(format!(
                "invalid lsp.diagnostics-severity-floor: expected one of 'error', 'warning', 'info', 'hint', got '{s}'"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StoredDiag {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) severity: DiagSeverity,
    pub(crate) message: String,
    pub(crate) code: Option<String>,
    pub(crate) source: Option<String>,
    /// The original wire-shaped `Diagnostic` (`textDocument/codeAction`
    /// needs to echo this back verbatim as `context.diagnostics` — the
    /// server's quickfixes are gated on the client showing the diagnostic
    /// it's fixing, and rebuilding this from `start`/`end`'s char offsets
    /// would mean Steel fabricating wire positions itself, which the
    /// encoding-safety rule forbids).
    pub(crate) raw: serde_json::Value,
}

#[derive(Default)]
pub(crate) struct DiagnosticsStore {
    by_buffer: HashMap<BufferId, Vec<(ServerId, Vec<StoredDiag>)>>,
    /// Bumped on every ingest or remap — cheap "did anything change" signal
    /// for Steel-side consumers (`on-diagnostics-changed`).
    pub(crate) generation: u64,
}

impl DiagnosticsStore {
    /// Replaces one server's diagnostics for `bid` (already coalesced —
    /// the caller keeps only the last `publishDiagnostics` per (server,
    /// uri) within a drain batch). `diags` must already be sorted by
    /// `start` — callers build it that way at ingest.
    pub(crate) fn replace(&mut self, server: ServerId, bid: BufferId, diags: Vec<StoredDiag>) {
        let entry = self.by_buffer.entry(bid).or_default();
        match entry.iter_mut().find(|(sid, _)| *sid == server) {
            Some(slot) => slot.1 = diags,
            None => entry.push((server, diags)),
        }
        self.generation += 1;
    }

    /// Remaps every stored range for `bid` through `cs` — must be called
    /// for every `ChangeSet` applied to an attached buffer, including
    /// undo/redo (same chokepoint as `flush_lsp_pending_changes`,
    /// consuming the same `Buffer.lsp_pending` entries — same source, both
    /// consumers). A range collapsed to empty by a covering deletion is
    /// dropped, not kept as a zero-width entry.
    pub(crate) fn remap_through(&mut self, bid: BufferId, cs: &ChangeSet) {
        let Some(entry) = self.by_buffer.get_mut(&bid) else {
            return;
        };
        for (_server, diags) in entry.iter_mut() {
            if diags.is_empty() {
                continue;
            }
            let mut ranges: Vec<(usize, usize)> = diags.iter().map(|d| (d.start, d.end)).collect();
            cs.map_ranges(&mut ranges);
            let mut idx = 0;
            diags.retain_mut(|d| {
                let (start, end) = ranges[idx];
                idx += 1;
                if end <= start {
                    false // collapsed by a covering deletion — drop
                } else {
                    d.start = start;
                    d.end = end;
                    true
                }
            });
            debug_assert!(
                diags.windows(2).all(|w| w[0].start <= w[1].start),
                "map_ranges must preserve sort order"
            );
        }
        self.generation += 1;
    }

    /// Drops every `StoredDiag` published by `server` — called when a
    /// server is stopped (`lsp_stop_one`) so its diagnostics don't survive
    /// the stop (drifting silently, since `remap_through` only runs for
    /// buffers still attached to a server) or duplicate a fresh instance's
    /// entry after `:lsp-restart` (a new `ServerId` would otherwise coexist
    /// with the old, frozen one via `replace`'s "push if no matching sid"
    /// path). A buffer left with no remaining server entry is dropped from
    /// `by_buffer` entirely, not kept as an empty `Vec`. Returns the buffers
    /// actually touched, so the caller can fire `OnDiagnosticsChanged` for
    /// exactly those — same "only the buffers this batch touched" discipline
    /// as `drain_lsp`'s `publishDiagnostics` ingest.
    pub(crate) fn remove_server(&mut self, server: ServerId) -> Vec<BufferId> {
        let mut touched = Vec::new();
        self.by_buffer.retain(|&bid, entry| {
            let before = entry.len();
            entry.retain(|(sid, _)| *sid != server);
            if entry.len() != before {
                touched.push(bid);
            }
            !entry.is_empty()
        });
        if !touched.is_empty() {
            self.generation += 1;
        }
        touched
    }

    /// Drops every diagnostic for `bid`, across every server — called when
    /// the buffer is closed (a pure memory-leak fix there: `BufferId` is a
    /// versioned slotmap key, so a future slot reuse can never alias with
    /// the closed buffer's stale entry) and on `:e!` reload (where it *is*
    /// a correctness fix — offsets computed against the pre-reload text
    /// must not survive against the new content). Returns whether anything
    /// was actually removed, so a reload caller only fires
    /// `OnDiagnosticsChanged` when the display actually changes.
    pub(crate) fn remove_buffer(&mut self, bid: BufferId) -> bool {
        let removed = self.by_buffer.remove(&bid).is_some();
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// Production callers: `:lsp-status` and the `(diagnostic-counts …)` builtin.
    pub(crate) fn counts(&self, bid: BufferId) -> (usize, usize) {
        let Some(entry) = self.by_buffer.get(&bid) else {
            return (0, 0);
        };
        let mut errors = 0;
        let mut warnings = 0;
        for (_server, diags) in entry {
            for d in diags {
                match d.severity {
                    DiagSeverity::Error => errors += 1,
                    DiagSeverity::Warning => warnings += 1,
                    DiagSeverity::Info | DiagSeverity::Hint => {}
                }
            }
        }
        (errors, warnings)
    }

    /// Production caller: the `(diagnostics-for-buffer …)` builtin. The
    /// underline/sign providers also read from here.
    ///
    /// Each server's own `Vec` is sorted by `start`, but with 2+ servers
    /// publishing for the same buffer, concatenating them in server order
    /// would not be globally sorted — callers that assume start-ascending
    /// order (e.g. `goto-next-diagnostic`'s nearest-match logic) would jump
    /// to whichever server happened to be iterated first rather than the
    /// nearest diagnostic. Collected and sorted once here so every caller
    /// gets a globally ordered result without re-deriving it.
    pub(crate) fn for_range(
        &self,
        bid: BufferId,
        range: Range<usize>,
        floor: DiagSeverity,
    ) -> impl Iterator<Item = &StoredDiag> {
        let (lo, hi) = (range.start, range.end);
        let mut out: Vec<&StoredDiag> = self
            .by_buffer
            .get(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter())
            .flat_map(move |(_server, diags)| {
                // Each server's Vec is sorted by `start` (see `replace` and
                // `remap_through`), so everything past the first `start >= hi`
                // can't overlap `range`. `end` isn't sorted, so the lower bound
                // still needs a full scan from the front.
                let upper = diags.partition_point(|d| d.start < hi);
                diags[..upper].iter()
            })
            .filter(move |d| d.severity <= floor && d.end > lo)
            .collect();
        out.sort_by_key(|d| d.start);
        out.into_iter()
    }
}

fn map_severity(sev: Option<lsp_types::DiagnosticSeverity>) -> DiagSeverity {
    match sev {
        // Spec: absent severity is left to the client to interpret — Error
        // keeps it maximally visible rather than silently downgrading it.
        None | Some(lsp_types::DiagnosticSeverity::ERROR) => DiagSeverity::Error,
        Some(lsp_types::DiagnosticSeverity::WARNING) => DiagSeverity::Warning,
        Some(lsp_types::DiagnosticSeverity::HINT) => DiagSeverity::Hint,
        // INFORMATION and any future/unknown severity value.
        Some(_) => DiagSeverity::Info,
    }
}

/// Widens a zero-length `[pos, pos)` range to one char — HUME diagnostic
/// decorations, like selections, are never empty. Widens forward by
/// default; widens backward instead when `pos` is at end-of-line or
/// end-of-buffer, so the range never crosses into the next line. Always
/// succeeds: the buffer invariant (`len_chars() >= 1`, always ending in a
/// structural `\n`) guarantees at least the newline itself to widen onto,
/// even on the minimal `"\n"` buffer — matching how a selection can cover
/// that same newline cell.
fn widen_zero_length(rope: &Rope, pos: usize) -> (usize, usize) {
    let len = rope.len_chars();
    if pos < len && rope.char(pos) != '\n' {
        (pos, pos + 1)
    } else if pos > 0 {
        (pos - 1, pos)
    } else {
        (0, 1)
    }
}

impl Editor {
    /// Ingests one already-coalesced `publishDiagnostics` payload (raw
    /// JSON — the caller kept only the last one per (server, uri) within
    /// this drain batch). Drops silently (one Trace line) when the URI
    /// doesn't resolve to an open buffer — v1 never opens a buffer just to
    /// hold diagnostics. Returns the buffer actually ingested into, so the
    /// caller can fire `OnDiagnosticsChanged` once per touched buffer
    /// — `None` on any drop path.
    pub(in crate::editor) fn ingest_publish_diagnostics(
        &mut self,
        server_id: ServerId,
        params: serde_json::Value,
    ) -> Option<BufferId> {
        let parsed: PublishDiagnosticsParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                self.report(
                    Severity::Trace,
                    format!("lsp: malformed publishDiagnostics: {e}"),
                );
                return None;
            }
        };

        let Ok(path) = hume_lsp::uri::uri_to_path(&parsed.uri) else {
            self.report(
                Severity::Trace,
                "lsp: publishDiagnostics with an unresolvable URI".to_string(),
            );
            return None;
        };
        let Ok(canonical) = path.canonicalize() else {
            self.report(
                Severity::Trace,
                format!(
                    "lsp: publishDiagnostics for an unknown file: {}",
                    path.display()
                ),
            );
            return None;
        };
        let Some(bid) = self.state.buffers.find_by_path(&canonical) else {
            self.report(
                Severity::Trace,
                format!(
                    "lsp: publishDiagnostics for an unopened buffer: {}",
                    canonical.display()
                ),
            );
            return None;
        };

        // A publish computed against an older version would convert its
        // positions against text that has since moved on — the server has
        // already received our newer didChange(s) and will republish
        // against the current version shortly. Drop it rather than store
        // positions that are quietly wrong until then; the existing
        // (already-remapped) stored diagnostics keep displaying meanwhile.
        // Absent version is always ingested (older/simpler servers omit it).
        if let Some(v) = parsed.version
            && v != wire_version(self.state.buffers.get(bid).text_gen)
        {
            self.report(
                Severity::Trace,
                format!("lsp: dropping publishDiagnostics for a stale version ({v})"),
            );
            return None;
        }

        let encoding = self
            .lsp
            .servers
            .get(&server_id)
            .map(|e| e.client.encoding)
            .unwrap_or(hume_editing::PositionEncoding::Utf16);
        let rope = self.state.buffers.get(bid).text().rope().clone();

        let mut stored: Vec<StoredDiag> = parsed
            .diagnostics
            .into_iter()
            .map(|d| {
                let raw = serde_json::to_value(&d).unwrap_or(serde_json::Value::Null);
                let start = wire_to_char(
                    &rope,
                    d.range.start.line as usize,
                    d.range.start.character as usize,
                    encoding,
                );
                let end = wire_to_char(
                    &rope,
                    d.range.end.line as usize,
                    d.range.end.character as usize,
                    encoding,
                );
                let (start, end) = if start == end {
                    widen_zero_length(&rope, start)
                } else {
                    (start, end)
                };
                StoredDiag {
                    start,
                    end,
                    severity: map_severity(d.severity),
                    message: d.message,
                    code: d.code.map(|c| match c {
                        lsp_types::NumberOrString::Number(n) => n.to_string(),
                        lsp_types::NumberOrString::String(s) => s,
                    }),
                    source: d.source,
                    raw,
                }
            })
            .collect();
        stored.sort_by_key(|d| d.start);

        self.lsp.diagnostics.replace(server_id, bid, stored);
        Some(bid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hume_editing::ChangeSetBuilder;
    use hume_engine::pipeline::EngineView;
    use hume_engine::theme::Theme;

    fn make_bid() -> BufferId {
        let mut ev = EngineView::new(Theme::default());
        ev.buffers
            .insert(hume_engine::pipeline::SharedBuffer::new())
    }

    /// Two guaranteed-distinct `BufferId`s — `make_bid()` calls each start a
    /// fresh `EngineView` with its own slotmap, so two separate calls are
    /// *not* guaranteed distinct (both can land on the same first-insert
    /// key). Needed by tests that must tell "this buffer" from "some other
    /// buffer" apart.
    fn make_two_bids() -> (BufferId, BufferId) {
        let mut ev = EngineView::new(Theme::default());
        let a = ev
            .buffers
            .insert(hume_engine::pipeline::SharedBuffer::new());
        let b = ev
            .buffers
            .insert(hume_engine::pipeline::SharedBuffer::new());
        (a, b)
    }

    fn diag(start: usize, end: usize, severity: DiagSeverity) -> StoredDiag {
        StoredDiag {
            start,
            end,
            severity,
            message: "boom".to_string(),
            code: None,
            source: None,
            raw: serde_json::Value::Null,
        }
    }

    #[test]
    fn counts_tallies_errors_and_warnings_only() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(
            ServerId(0),
            bid,
            vec![
                diag(0, 1, DiagSeverity::Error),
                diag(2, 3, DiagSeverity::Error),
                diag(4, 5, DiagSeverity::Warning),
                diag(6, 7, DiagSeverity::Info),
                diag(8, 9, DiagSeverity::Hint),
            ],
        );
        assert_eq!(store.counts(bid), (2, 1));
    }

    #[test]
    fn counts_is_zero_for_an_unknown_buffer() {
        let store = DiagnosticsStore::default();
        assert_eq!(store.counts(make_bid()), (0, 0));
    }

    #[test]
    fn remove_server_clears_that_servers_diagnostics_only() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
        store.replace(ServerId(1), bid, vec![diag(2, 3, DiagSeverity::Warning)]);

        let touched = store.remove_server(ServerId(0));
        assert_eq!(touched, vec![bid]);
        assert_eq!(
            store.counts(bid),
            (0, 1),
            "server 1's diagnostic must survive"
        );
    }

    #[test]
    fn remove_server_drops_the_buffer_entry_once_no_server_remains() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);

        store.remove_server(ServerId(0));
        assert_eq!(store.counts(bid), (0, 0));
        assert!(
            store
                .for_range(bid, 0..100, DiagSeverity::Hint)
                .next()
                .is_none(),
            "no entry should remain for a buffer with no servers left"
        );
    }

    #[test]
    fn for_range_is_globally_sorted_across_multiple_servers() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        // Server 0 (inserted first) publishes a diagnostic starting later;
        // server 1 (inserted after) publishes one starting earlier —
        // concatenating in insertion order would put the later one first.
        store.replace(ServerId(0), bid, vec![diag(10, 12, DiagSeverity::Error)]);
        store.replace(ServerId(1), bid, vec![diag(0, 2, DiagSeverity::Warning)]);

        let starts: Vec<usize> = store
            .for_range(bid, 0..100, DiagSeverity::Hint)
            .map(|d| d.start)
            .collect();
        assert_eq!(
            starts,
            vec![0, 10],
            "results must be globally start-ascending regardless of server insertion order"
        );
    }

    #[test]
    fn remove_server_is_a_no_op_for_a_server_with_nothing_stored() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
        let gen_before = store.generation;

        let touched = store.remove_server(ServerId(99));
        assert!(touched.is_empty());
        assert_eq!(
            store.generation, gen_before,
            "no change must not bump generation"
        );
        assert_eq!(
            store.counts(bid),
            (1, 0),
            "unrelated server's diagnostics must survive"
        );
    }

    #[test]
    fn remove_buffer_clears_every_servers_diagnostics_for_that_buffer() {
        let mut store = DiagnosticsStore::default();
        let (bid, other_bid) = make_two_bids();
        store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
        store.replace(ServerId(1), bid, vec![diag(2, 3, DiagSeverity::Warning)]);
        store.replace(
            ServerId(0),
            other_bid,
            vec![diag(0, 1, DiagSeverity::Error)],
        );

        store.remove_buffer(bid);

        assert_eq!(
            store.counts(bid),
            (0, 0),
            "every server's entry for bid must be gone"
        );
        assert_eq!(
            store.counts(other_bid),
            (1, 0),
            "an unrelated buffer's diagnostics must survive"
        );
    }

    #[test]
    fn for_range_respects_severity_floor() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(
            ServerId(0),
            bid,
            vec![
                diag(0, 1, DiagSeverity::Error),
                diag(2, 3, DiagSeverity::Warning),
                diag(4, 5, DiagSeverity::Info),
            ],
        );
        let kept: Vec<DiagSeverity> = store
            .for_range(bid, 0..100, DiagSeverity::Warning)
            .map(|d| d.severity)
            .collect();
        assert_eq!(kept, vec![DiagSeverity::Error, DiagSeverity::Warning]);
    }

    #[test]
    fn for_range_respects_range_bounds() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(
            ServerId(0),
            bid,
            vec![
                diag(0, 5, DiagSeverity::Error),
                diag(10, 15, DiagSeverity::Error),
                diag(20, 25, DiagSeverity::Error),
            ],
        );
        let kept: Vec<(usize, usize)> = store
            .for_range(bid, 8..18, DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
            .collect();
        assert_eq!(kept, vec![(10, 15)]);
    }

    #[test]
    fn for_range_keeps_a_diagnostic_that_starts_before_the_range_but_overlaps_it() {
        // Regression test for the partition_point optimization in `for_range`:
        // the inner Vec is sorted by `start`, not `end`, so a diagnostic that
        // starts before the queried range can still overlap it and must not
        // be dropped by the upper-bound cut.
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(0, 10, DiagSeverity::Error)]);

        let kept: Vec<(usize, usize)> = store
            .for_range(bid, 8..18, DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
            .collect();
        assert_eq!(
            kept,
            vec![(0, 10)],
            "a diagnostic starting before the range must survive if it overlaps"
        );
    }

    #[test]
    fn remap_insert_before_shifts_the_range() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

        let mut b = ChangeSetBuilder::new(20);
        b.retain(0).insert("XXX").retain_rest();
        store.remap_through(bid, &b.finish());

        let kept: Vec<(usize, usize)> = store
            .for_range(bid, 0..100, DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
            .collect();
        assert_eq!(
            kept,
            vec![(13, 18)],
            "an insert before the range shifts it forward"
        );
    }

    #[test]
    fn remap_insert_inside_grows_the_range() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

        let mut b = ChangeSetBuilder::new(20);
        b.retain(12).insert("XX").retain_rest();
        store.remap_through(bid, &b.finish());

        let kept: Vec<(usize, usize)> = store
            .for_range(bid, 0..100, DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
            .collect();
        assert_eq!(kept, vec![(10, 17)], "an insert inside the range grows it");
    }

    #[test]
    fn remap_insert_after_leaves_the_range_unchanged() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

        let mut b = ChangeSetBuilder::new(20);
        b.retain(18).insert("XX").retain_rest();
        store.remap_through(bid, &b.finish());

        let kept: Vec<(usize, usize)> = store
            .for_range(bid, 0..100, DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
            .collect();
        assert_eq!(
            kept,
            vec![(10, 15)],
            "an insert after the range must not move it"
        );
    }

    #[test]
    fn remap_deletion_covering_the_range_drops_it() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

        let mut b = ChangeSetBuilder::new(20);
        b.retain(5).delete(15).retain_rest();
        store.remap_through(bid, &b.finish());

        let kept: Vec<(usize, usize)> = store
            .for_range(bid, 0..100, DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
            .collect();
        assert!(
            kept.is_empty(),
            "a deletion covering the range must drop it, not zero it"
        );
    }

    #[test]
    fn remap_bumps_generation_only_when_the_buffer_has_stored_diagnostics() {
        let mut store = DiagnosticsStore::default();
        let bid = make_bid();
        let mut b = ChangeSetBuilder::new(5);
        b.retain(0).insert("X").retain_rest();
        let cs = b.finish();

        let gen_before = store.generation;
        store.remap_through(bid, &cs); // no entry for bid — no-op
        assert_eq!(store.generation, gen_before);

        store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
        let gen_after_replace = store.generation;
        store.remap_through(bid, &cs);
        assert_eq!(store.generation, gen_after_replace + 1);
    }

    #[test]
    fn map_severity_absent_defaults_to_error() {
        assert_eq!(map_severity(None), DiagSeverity::Error);
    }

    #[test]
    fn map_severity_maps_the_known_wire_values() {
        assert_eq!(
            map_severity(Some(lsp_types::DiagnosticSeverity::ERROR)),
            DiagSeverity::Error
        );
        assert_eq!(
            map_severity(Some(lsp_types::DiagnosticSeverity::WARNING)),
            DiagSeverity::Warning
        );
        assert_eq!(
            map_severity(Some(lsp_types::DiagnosticSeverity::INFORMATION)),
            DiagSeverity::Info
        );
        assert_eq!(
            map_severity(Some(lsp_types::DiagnosticSeverity::HINT)),
            DiagSeverity::Hint
        );
    }

    #[test]
    fn widen_zero_length_widens_forward_mid_line() {
        let rope = Rope::from_str("hello\n");
        assert_eq!(widen_zero_length(&rope, 2), (2, 3));
    }

    #[test]
    fn widen_zero_length_widens_backward_at_end_of_line() {
        let rope = Rope::from_str("hello\n");
        // Position 5 is the '\n' — widening forward would cross the line
        // boundary, so it must widen backward instead.
        assert_eq!(widen_zero_length(&rope, 5), (4, 5));
    }

    #[test]
    fn widen_zero_length_widens_backward_at_end_of_buffer() {
        let rope = Rope::from_str("hi");
        assert_eq!(widen_zero_length(&rope, 2), (1, 2));
    }

    /// On the minimal 1-char "\n" buffer, `pos = 0` has no char to widen
    /// onto in either direction under the general rule — it must widen onto
    /// the structural newline itself rather than staying `(0, 0)`.
    #[test]
    fn widen_zero_length_widens_onto_the_newline_on_the_minimal_buffer() {
        let rope = Rope::from_str("\n");
        assert_eq!(widen_zero_length(&rope, 0), (0, 1));
    }
}
