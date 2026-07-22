//! Diagnostics store: `publishDiagnostics` lands here, converted to char
//! offsets at ingest, coalesced per drain batch, remapped through every
//! subsequent edit. Bulk never reaches Steel — Steel gets
//! a signal + bounded pulls.

use rustc_hash::FxHashMap;
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
    /// The original wire-shaped `Diagnostic`, serialized back from the
    /// parsed `lsp_types::Diagnostic` (`textDocument/codeAction` needs to
    /// echo this back verbatim as `context.diagnostics` — the server's
    /// quickfixes are gated on the client showing the diagnostic it's
    /// fixing, and rebuilding this from `start`/`end`'s char offsets would
    /// mean Steel fabricating wire positions itself, which the
    /// encoding-safety rule forbids). The roundtrip preserves every spec
    /// field, including `data` (some servers need it echoed back for
    /// `codeAction` too).
    pub(crate) raw: serde_json::Value,
}

#[derive(Default)]
pub(crate) struct DiagnosticsStore {
    by_buffer: FxHashMap<BufferId, Vec<(ServerId, Vec<StoredDiag>)>>,
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
    /// Ingests one already-coalesced, already-classified `publishDiagnostics`
    /// payload (the caller kept only the last one per (server, uri) within
    /// this drain batch — a malformed payload never reaches here, since
    /// `hume-lsp` classifies it as a `ServerNotification` fallthrough
    /// instead). Drops silently (one Trace line) when the URI doesn't
    /// resolve to an open buffer — v1 never opens a buffer just to hold
    /// diagnostics. Returns the buffer actually ingested into, so the caller
    /// can fire `OnDiagnosticsChanged` once per touched buffer — `None` on
    /// any drop path.
    pub(in crate::editor) fn ingest_publish_diagnostics(
        &mut self,
        server_id: ServerId,
        parsed: PublishDiagnosticsParams,
    ) -> Option<BufferId> {
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
            .map(|e| e.client.encoding())
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
mod tests;
