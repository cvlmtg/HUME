//! Persistent message log.
//!
//! The [`MessageLog`] accumulates [`LogEntry`] values produced during an editing
//! session — config warnings, scripting errors, plugin conflicts. Entries survive
//! keypresses and can be reviewed at any time via `:messages`.

use std::collections::VecDeque;

use super::EditorState;

// ── Severity ─────────────────────────────────────────────────────────────────

/// Severity level for a message, controlling both logging and display.
///
/// | Severity | Logged? | Shown as `status_msg`? |
/// |----------|---------|------------------------|
/// | Info     | No      | Yes                    |
/// | Warning  | Yes     | Yes                    |
/// | Error    | Yes     | Yes                    |
/// | Trace    | Yes     | No                     |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    /// Ephemeral confirmation (e.g. "Written 42 lines"). Shown, not logged.
    Info,
    /// Something the user should review (e.g. unknown config key). Logged and shown.
    Warning,
    /// A failure the user must address (e.g. script error). Logged and shown.
    Error,
    /// Verbose diagnostic detail (e.g. stack trace). Logged only, not shown in statusline.
    Trace,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Trace => "trace",
        }
    }
}

// ── LogEntry ─────────────────────────────────────────────────────────────────

/// A single entry in the persistent message log.
#[derive(Debug, Clone)]
pub(crate) struct LogEntry {
    pub(crate) severity: Severity,
    pub(crate) text: String,
}

// ── MessageLog ───────────────────────────────────────────────────────────────

/// Maximum number of entries kept in the log.
///
/// When the cap is exceeded the oldest entry is evicted. `VecDeque` makes
/// both the push and the eviction O(1) amortized, so a misbehaving plugin
/// flooding the log cannot degrade performance.
const MAX_ENTRIES: usize = 1000;

/// Persistent, append-only log of messages from the current editing session.
///
/// Entries accumulate until the session ends; `mark_all_seen` tracks which
/// have been reviewed via `:messages`. New entries after a mark bump the
/// unseen count again, prompting the user to check.
pub(crate) struct MessageLog {
    // VecDeque so pop_front() (eviction) and push_back() (append) are both
    // O(1) amortized — a Vec would shift all elements on every eviction.
    entries: VecDeque<LogEntry>,
    /// Index of the first unseen entry. Everything at `index >= seen_up_to` is
    /// "unread". Updated by `mark_all_seen()`.
    seen_up_to: usize,
}

impl MessageLog {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            seen_up_to: 0,
        }
    }

    /// Append an entry to the log. Called by `Editor::report`.
    ///
    /// When the entry count would exceed [`MAX_ENTRIES`], the oldest entry is
    /// evicted and `seen_up_to` is shifted so it stays in bounds.
    pub(crate) fn push(&mut self, severity: Severity, text: String) {
        if self.entries.len() == MAX_ENTRIES {
            self.entries.pop_front();
            self.seen_up_to = self.seen_up_to.saturating_sub(1);
        }
        self.entries.push_back(LogEntry { severity, text });
    }

    /// All entries in chronological order. Used only in tests.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> impl ExactSizeIterator<Item = &LogEntry> {
        self.entries.iter()
    }

    /// Whether there are any entries that have not been seen via `:messages`.
    pub(crate) fn has_unseen(&self) -> bool {
        self.seen_up_to < self.entries.len()
    }

    /// Count of unseen entries by severity: `(errors, warnings)`.
    ///
    /// `Info` entries are never logged; `Trace` entries are not surfaced in
    /// the summary because they are supplemental detail, not actionable items.
    pub(crate) fn unseen_counts(&self) -> (usize, usize) {
        self.entries
            .iter()
            .skip(self.seen_up_to)
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
    pub(crate) fn mark_all_seen(&mut self) {
        self.seen_up_to = self.entries.len();
    }

    /// One-line summary shown in the statusline when there are unseen
    /// `Warning` or `Error` entries.
    ///
    /// Returns `None` when everything has been seen, *or* when the only unseen
    /// entries are `Trace` — trace messages are logged for `:messages` review
    /// but never raise a statusline indicator (see [`Severity`] table).
    pub(crate) fn summary_text(&self) -> Option<String> {
        if !self.has_unseen() {
            return None;
        }
        let (errors, warnings) = self.unseen_counts();
        let msg = match (errors, warnings) {
            (0, 0) => return None,
            (e, 0) => {
                let noun = if e == 1 { "error" } else { "errors" };
                format!("{e} {noun} — :messages for details")
            }
            (0, w) => {
                let noun = if w == 1 { "warning" } else { "warnings" };
                format!("{w} {noun} — :messages for details")
            }
            (e, w) => {
                let e_noun = if e == 1 { "error" } else { "errors" };
                let w_noun = if w == 1 { "warning" } else { "warnings" };
                format!("{e} {e_noun}, {w} {w_noun} — :messages for details")
            }
        };
        Some(msg)
    }

    /// Full log formatted for display in the `:messages` scratch buffer.
    ///
    /// Each line is prefixed with `[severity]` for scannability. Returns an
    /// empty string if there are no entries.
    pub(crate) fn format_for_display(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for entry in &self.entries {
            out.push('[');
            out.push_str(entry.severity.label());
            out.push_str("] ");
            out.push_str(&entry.text);
            out.push('\n');
        }
        out
    }
}

impl EditorState {
    // ── Status messages ───────────────────────────────────────────────────────

    /// Record a status message / warning / error on this state.
    ///
    /// Called by EditorCmd handlers that only have `&mut EditorState` access.
    /// The `Editor::report` method delegates here.
    pub(super) fn report(&mut self, severity: Severity, text: String) {
        match severity {
            Severity::Info => {
                self.status_msg = Some(text);
            }
            Severity::Warning | Severity::Error => {
                self.message_log.push(severity, text.clone());
                self.status_msg = Some(text);
            }
            Severity::Trace => {
                self.message_log.push(severity, text);
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
