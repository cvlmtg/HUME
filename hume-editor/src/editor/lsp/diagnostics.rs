//! Diagnostics store: `publishDiagnostics` lands here, converted to char
//! offsets at ingest, coalesced per drain batch, remapped through every
//! subsequent edit. Bulk never reaches Steel (hub guardrail) — Steel gets
//! a signal + bounded pulls (B5).

use std::collections::HashMap;
use std::ops::Range;

use hume_editing::changeset::ChangeSet;
use hume_editing::wire_to_char;
use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use lsp_types::PublishDiagnosticsParams;
use ropey::Rope;

use crate::editor::Editor;
use crate::editor::message_log::Severity;

/// Ordered least-to-most-lenient so `severity <= floor` means "at least as
/// severe as floor" — e.g. `floor = Warning` keeps `Error` and `Warning`,
/// drops `Info`/`Hint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DiagSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredDiag {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) severity: DiagSeverity,
    // Read by future consumers (U1 hover text, U6 drawer list, C10's
    // :lsp-status) — stored now so ingest doesn't need re-visiting later.
    #[allow(dead_code)]
    pub(crate) message: String,
    #[allow(dead_code)]
    pub(crate) code: Option<String>,
    #[allow(dead_code)]
    pub(crate) source: Option<String>,
}

#[derive(Default)]
pub(crate) struct DiagnosticsStore {
    by_buffer: HashMap<BufferId, Vec<(ServerId, Vec<StoredDiag>)>>,
    /// Bumped on every ingest or remap — cheap "did anything change" signal
    /// for Steel-side consumers (B7's `on-diagnostics-changed`).
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
    /// undo/redo (same chokepoint as C7's `flush_lsp_pending_changes`,
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

    /// No production caller until C10's `:lsp-status` / B5's Steel builtin.
    #[allow(dead_code)]
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

    /// No production caller until U1/U2 (underline/sign providers) or B5's
    /// `diagnostics-for-buffer` builtin.
    #[allow(dead_code)]
    pub(crate) fn for_range(
        &self,
        bid: BufferId,
        range: Range<usize>,
        floor: DiagSeverity,
    ) -> impl Iterator<Item = &StoredDiag> {
        self.by_buffer
            .get(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter())
            .flat_map(|(_server, diags)| diags.iter())
            .filter(move |d| d.severity <= floor && d.start < range.end && d.end > range.start)
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
/// end-of-buffer, so the range never crosses into the next line.
fn widen_zero_length(rope: &Rope, pos: usize) -> (usize, usize) {
    let len = rope.len_chars();
    if pos < len && rope.char(pos) != '\n' {
        (pos, pos + 1)
    } else if pos > 0 {
        (pos - 1, pos)
    } else {
        (pos, pos)
    }
}

impl Editor {
    /// Ingests one already-coalesced `publishDiagnostics` payload (raw
    /// JSON — the caller kept only the last one per (server, uri) within
    /// this drain batch). Drops silently (one Trace line) when the URI
    /// doesn't resolve to an open buffer — v1 never opens a buffer just to
    /// hold diagnostics. Returns the buffer actually ingested into, so the
    /// caller can fire `OnDiagnosticsChanged` (B7) once per touched buffer
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
                format!("lsp: publishDiagnostics for an unknown file: {}", path.display()),
            );
            return None;
        };
        let Some(bid) = self.state.buffers.find_by_path(&canonical) else {
            self.report(
                Severity::Trace,
                format!("lsp: publishDiagnostics for an unopened buffer: {}", canonical.display()),
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
            && v != self.state.buffers.get(bid).text_gen as i32
        {
            self.report(
                Severity::Trace,
                format!("lsp: dropping publishDiagnostics for a stale version ({v})"),
            );
            return None;
        }

        let encoding = self
            .lsp
            .clients
            .get(&server_id)
            .map(|c| c.encoding)
            .unwrap_or(hume_editing::PositionEncoding::Utf16);
        let rope = self.state.buffers.get(bid).text().rope().clone();

        let mut stored: Vec<StoredDiag> = parsed
            .diagnostics
            .into_iter()
            .map(|d| {
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
        ev.buffers.insert(hume_engine::pipeline::SharedBuffer::new())
    }

    fn diag(start: usize, end: usize, severity: DiagSeverity) -> StoredDiag {
        StoredDiag {
            start,
            end,
            severity,
            message: "boom".to_string(),
            code: None,
            source: None,
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
        assert_eq!(kept, vec![(13, 18)], "an insert before the range shifts it forward");
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
        assert_eq!(kept, vec![(10, 15)], "an insert after the range must not move it");
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
        assert!(kept.is_empty(), "a deletion covering the range must drop it, not zero it");
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
}
