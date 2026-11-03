//! Persistent message log.
//!
//! The [`MessageLog`] accumulates [`LogEntry`] values produced during an editing
//! session: config warnings, scripting errors, plugin conflicts. Entries survive
//! keypresses and can be reviewed at any time via `:messages`.

use std::collections::VecDeque;

use hume_core::Severity;
use hume_engine::theme::diagnostic_scopes;

use super::error::CommandError;
use super::settings::MessagesMinSeverity;
use super::{Editor, EditorState};

/// A single entry in the persistent message log.
#[derive(Debug, Clone)]
pub(in crate::editor) struct LogEntry {
    pub(in crate::editor) severity: Severity,
    pub(in crate::editor) text: String,
}

// ── MessageLog ───────────────────────────────────────────────────────────────

/// Maximum number of entries kept in the log.
///
/// A push into a full log scans it once to pick the entry to evict, so it
/// costs O(`MAX_ENTRIES`), whatever floods the log.
const MAX_ENTRIES: usize = 1000;

/// Persistent, append-only log of messages from the current editing session.
///
/// Entries accumulate until the session ends; `mark_all_seen` tracks which
/// have been reviewed via `:messages`. New entries after a mark bump the
/// unseen count again, prompting the user to check.
pub(crate) struct MessageLog {
    // VecDeque so removing an entry near the front, where the oldest trace
    // usually is, shifts only the entries before it.
    entries: VecDeque<LogEntry>,
    /// Index of the first unseen entry. Everything at `index >= seen_up_to` is
    /// "unread". Updated by `mark_all_seen()`.
    seen_up_to: usize,
    /// Lifetime count of `Error`/`Warning` messages ever pushed: monotonic,
    /// never decremented by eviction (unlike `unseen_counts`, which reads the
    /// live `entries` deque and so *can* drop below a value it reported
    /// earlier once `MAX_ENTRIES` starts evicting). `:reload-config` diffs
    /// this against a before/after snapshot to detect "did this reload push a
    /// new warning or error", and `handle_input` to keep a command's message
    /// on the status line. Both ask what was reported, whether or not the
    /// floor shows it.
    total_errors: u64,
    total_warnings: u64,
    /// The `messages-min-severity` setting, kept in step by its resync. It
    /// decides what is shown, never what is stored.
    floor: MessagesMinSeverity,
}

impl MessageLog {
    pub(in crate::editor) fn new(floor: MessagesMinSeverity) -> Self {
        Self {
            entries: VecDeque::new(),
            seen_up_to: 0,
            total_errors: 0,
            total_warnings: 0,
            floor,
        }
    }

    /// Show the stored entries at or above `floor`, those pushed before this
    /// call included.
    pub(in crate::editor) fn set_floor(&mut self, floor: MessagesMinSeverity) {
        self.floor = floor;
    }

    /// Append an entry to the log, whatever the floor.
    ///
    /// When the entry count would exceed [`MAX_ENTRIES`], the oldest trace is
    /// evicted, else the oldest warning, else the oldest error, and
    /// `seen_up_to` moves back when the evicted entry was a seen one.
    pub(in crate::editor) fn push(&mut self, severity: Severity, text: String) {
        match severity {
            Severity::Error => self.total_errors += 1,
            Severity::Warning => self.total_warnings += 1,
            Severity::Info | Severity::Trace => {}
        }
        if self.entries.len() == MAX_ENTRIES
            && let Some(index) = self.eviction_index()
        {
            self.entries.remove(index);
            if index < self.seen_up_to {
                self.seen_up_to -= 1;
            }
        }
        self.entries.push_back(LogEntry { severity, text });
    }

    /// The oldest entry of the least severe kind present.
    fn eviction_index(&self) -> Option<usize> {
        let rank = |severity| match severity {
            Severity::Info | Severity::Trace => 0,
            Severity::Warning => 1,
            Severity::Error => 2,
        };
        self.entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| rank(entry.severity))
            .map(|(index, _)| index)
    }

    /// The entries from index `start` on that the floor shows.
    pub(in crate::editor::message_log) fn shown_from(
        &self,
        start: usize,
    ) -> impl Iterator<Item = &LogEntry> {
        self.entries
            .iter()
            .skip(start)
            .filter(|entry| self.floor.shows(entry.severity))
    }

    /// Lifetime `(errors, warnings)` pushed so far, evicted or not. See the
    /// field docs for why this, not `unseen_counts`, is the eviction-proof
    /// way to detect "were any new warnings/errors reported between two
    /// points in time".
    pub(crate) fn totals(&self) -> (u64, u64) {
        (self.total_errors, self.total_warnings)
    }

    /// Writes every warning and error still in the log to `w`, oldest first,
    /// one `hume: <severity>: <text>` line each. This is how headless mode,
    /// which has no statusline or `:messages`, reports them.
    pub(crate) fn write_problems(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        for entry in &self.entries {
            if matches!(entry.severity, Severity::Warning | Severity::Error) {
                writeln!(w, "hume: {}: {}", entry.severity.name(), entry.text)?;
            }
        }
        Ok(())
    }

    /// All entries in chronological order. Used only in tests.
    #[cfg(test)]
    pub(in crate::editor) fn entries(&self) -> impl ExactSizeIterator<Item = &LogEntry> {
        self.entries.iter()
    }

    /// Whether there are any entries that have not been seen via `:messages`.
    pub(in crate::editor) fn has_unseen(&self) -> bool {
        self.seen_up_to < self.entries.len()
    }

    /// Count of unseen entries the floor shows, by severity: `(errors,
    /// warnings)`.
    ///
    /// `Info` entries are never logged; `Trace` entries are not surfaced in
    /// the summary because they are supplemental detail, not actionable items.
    pub(in crate::editor::message_log) fn unseen_counts(&self) -> (usize, usize) {
        self.shown_from(self.seen_up_to)
            .fold((0, 0), |(e, w), entry| match entry.severity {
                Severity::Error => (e + 1, w),
                Severity::Warning => (e, w + 1),
                _ => (e, w),
            })
    }

    /// Mark all current entries as seen.
    ///
    /// Called when the user opens `:messages`, or automatically after the
    /// statusline summary's keystroke budget elapses.
    pub(in crate::editor) fn mark_all_seen(&mut self) {
        self.seen_up_to = self.entries.len();
    }

    /// One-line summary shown in the statusline when there are unseen
    /// `Warning` or `Error` entries.
    ///
    /// Returns `None` when everything has been seen, *or* when no unseen
    /// entry the floor shows is a `Warning` or `Error`: trace messages never
    /// raise a statusline indicator (see [`Severity`] table).
    pub(crate) fn summary_text(&self) -> Option<String> {
        if !self.has_unseen() {
            return None;
        }
        let (errors, warnings) = self.unseen_counts();
        let msg = match (errors, warnings) {
            (0, 0) => return None,
            (e, 0) => {
                let noun = if e == 1 { "error" } else { "errors" };
                format!("{e} {noun} (see :messages)")
            }
            (0, w) => {
                let noun = if w == 1 { "warning" } else { "warnings" };
                format!("{w} {noun} (see :messages)")
            }
            (e, w) => {
                let e_noun = if e == 1 { "error" } else { "errors" };
                let w_noun = if w == 1 { "warning" } else { "warnings" };
                format!("{e} {e_noun}, {w} {w_noun} (see :messages)")
            }
        };
        Some(msg)
    }

    /// The entries the floor shows, formatted for display in the `:messages`
    /// scratch buffer, along with the severity-highlight spans for each line.
    ///
    /// Each line is `[severity] text\n`. Returns an empty string and no
    /// spans if no entry is shown. Spans are `(start, end, scope name)`
    /// char offsets (not bytes), since message text may be non-ASCII. Scope names,
    /// not `ScopeId`s: this type has no registry access (it's not an
    /// `Editor` method), so interning is the caller's job. See
    /// `commands/typed_misc.rs`'s `typed_messages`, the sole production
    /// caller, which turns each name into an `ExtraHighlightEntry`.
    ///
    /// Highlighting goes through the `Extra` tier rather than tree-sitter:
    /// `[messages]` is a synthetic, path-less buffer, so `detect_language`
    /// never attaches a grammar to it (and a grammar for `[severity] text`
    /// lines would be absurd anyway).
    pub(in crate::editor) fn format_with_spans(
        &self,
    ) -> (String, Vec<(usize, usize, &'static str)>) {
        let mut out = String::new();
        let mut spans = Vec::new();
        let mut pos = 0;
        for entry in self.shown_from(0) {
            let label = entry.severity.name();

            out.push('[');
            out.push_str(label);
            out.push(']');
            let badge_len = label.len() + 2; // '[' + label + ']'
            spans.push((
                pos,
                pos + badge_len,
                diagnostic_scopes::message_badge(entry.severity),
            ));
            pos += badge_len;

            out.push(' ');
            pos += 1;

            let text_len = entry.text.chars().count();
            if text_len > 0 {
                out.push_str(&entry.text);
                spans.push((
                    pos,
                    pos + text_len,
                    diagnostic_scopes::message_text(entry.severity),
                ));
                pos += text_len;
            }

            out.push('\n');
            pos += 1;
        }
        (out, spans)
    }

    /// Test-only convenience over [`format_with_spans`] for assertions that
    /// only care about the rendered text.
    #[cfg(test)]
    pub(in crate::editor) fn format_for_display(&self) -> String {
        self.format_with_spans().0
    }
}

impl Editor {
    /// Report a message through [`EditorState::report`].
    pub(in crate::editor) fn report(&mut self, severity: Severity, text: impl Into<String>) {
        self.state.report(severity, text);
    }

    /// Report `e` at its own severity.
    pub(in crate::editor) fn report_error(&mut self, e: &CommandError) {
        self.state.report_error(e);
    }
}

impl EditorState {
    // ── Status messages ───────────────────────────────────────────────────────

    /// Report `e` at its own severity.
    pub(super) fn report_error(&mut self, e: &CommandError) {
        self.report(e.severity(), e.message());
    }

    /// Trace a write from `op` that a higher origin's `subject` left without
    /// effect.
    pub(in crate::editor) fn report_shadowed(&mut self, op: &str, subject: impl std::fmt::Display) {
        self.report(
            Severity::Trace,
            format!("{op}: a higher origin's {subject} applies"),
        );
    }

    /// Report a message, routing it based on severity:
    ///
    /// - `Info`    → set `status_msg` only (ephemeral, not logged)
    /// - `Warning` → push to `message_log` AND set `status_msg`
    /// - `Error`   → push to `message_log` AND set `status_msg`
    /// - `Trace`   → push to `message_log` only (not shown in statusline)
    pub(super) fn report(&mut self, severity: Severity, text: impl Into<String>) {
        match severity {
            Severity::Info => {
                self.status_msg = Some(text.into());
            }
            Severity::Warning | Severity::Error => {
                let text = text.into();
                self.message_log.push(severity, text.clone());
                self.status_msg = Some(text);
            }
            Severity::Trace => {
                self.message_log.push(severity, text.into());
            }
        }
    }
}

#[cfg(test)]
mod tests;
