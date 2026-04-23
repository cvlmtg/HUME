//! Persistent message log and scratch-buffer overlay.
//!
//! The [`MessageLog`] accumulates [`LogEntry`] values produced during an editing
//! session — config warnings, scripting errors, plugin conflicts. Entries survive
//! keypresses and can be reviewed at any time via `:messages`.
//!
//! The [`ScratchView`] is a read-only overlay that temporarily replaces the main
//! document in the editor's render and key-dispatch paths. Used by `:messages`;
//! designed to be reusable for `:help` and similar commands later.

use std::collections::VecDeque;

use crate::core::text::Text;
use crate::core::selection::{Selection, SelectionSet};

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
    ///
    /// No callers yet
    #[allow(dead_code)]
    Trace,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Info    => "info",
            Severity::Warning => "warning",
            Severity::Error   => "error",
            Severity::Trace   => "trace",
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
        Self { entries: VecDeque::new(), seen_up_to: 0 }
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
        self.entries.iter().skip(self.seen_up_to).fold((0, 0), |(e, w), entry| match entry.severity {
            Severity::Error   => (e + 1, w),
            Severity::Warning => (e, w + 1),
            _ => (e, w),
        })
    }

    /// Mark all current entries as seen. Called when the user opens `:messages`.
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

// ── ScratchView ──────────────────────────────────────────────────────────────

/// A read-only buffer overlay that temporarily replaces the main document view.
///
/// When `Editor::scratch_view` is `Some`, the engine renders this buffer instead
/// of `editor.doc`, and all keys are intercepted for navigation / dismissal.
///
/// Used by `:messages`; designed to be reusable for `:help` later.
pub(crate) struct ScratchView {
    /// The read-only content to display.
    pub(crate) buf: Text,
    /// Current cursor/scroll position within the scratch buffer.
    pub(crate) sels: SelectionSet,
    /// Label shown in the statusline FileName slot (e.g. `"[messages]"`).
    pub(crate) label: &'static str,
}

impl ScratchView {
    /// Build a scratch view from a multi-line string, cursor at the last line.
    pub(crate) fn from_text(text: &str, label: &'static str) -> Self {
        let buf = Text::from(text);
        // Position the cursor at the start of the last content line so the user
        // sees the most recent entries when the buffer opens.
        let last_line = buf.rope().len_lines().saturating_sub(2); // skip trailing \n line
        let last_char = buf.rope().line_to_char(last_line);
        let sels = SelectionSet::single(Selection::collapsed(last_char));
        Self { buf, sels, label }
    }

    /// Build a scratch view with the cursor placed at `line` (0-indexed).
    ///
    /// Used when the caller wants a specific row highlighted on open (e.g. `:ls`
    /// positions the cursor on the current buffer's row).  Out-of-bounds lines
    /// are clamped to the last content line.
    pub(crate) fn from_text_at_line(text: &str, label: &'static str, line: usize) -> Self {
        let buf = Text::from(text);
        let last_content = buf.rope().len_lines().saturating_sub(2);
        let target_line = line.min(last_content);
        let char_pos = buf.rope().line_to_char(target_line);
        let sels = SelectionSet::single(Selection::collapsed(char_pos));
        Self { buf, sels, label }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_log(entries: &[(Severity, &str)]) -> MessageLog {
        let mut log = MessageLog::new();
        for (sev, text) in entries {
            log.push(*sev, text.to_string());
        }
        log
    }

    #[test]
    fn push_and_entries() {
        let log = make_log(&[
            (Severity::Warning, "first"),
            (Severity::Error,   "second"),
        ]);
        let entries: Vec<_> = log.entries().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "first");
        assert_eq!(entries[1].severity, Severity::Error);
    }

    #[test]
    fn unseen_counts_initial() {
        let log = make_log(&[
            (Severity::Error,   "e1"),
            (Severity::Warning, "w1"),
            (Severity::Warning, "w2"),
            (Severity::Trace,   "t1"),
        ]);
        let (errors, warnings) = log.unseen_counts();
        assert_eq!(errors, 1);
        assert_eq!(warnings, 2);
        // Trace entries are not counted in the summary counts.
    }

    #[test]
    fn has_unseen_empty_log() {
        let log = MessageLog::new();
        assert!(!log.has_unseen());
    }

    #[test]
    fn mark_all_seen_clears_unseen() {
        let mut log = make_log(&[(Severity::Error, "e1"), (Severity::Warning, "w1")]);
        assert!(log.has_unseen());
        log.mark_all_seen();
        assert!(!log.has_unseen());
        let (e, w) = log.unseen_counts();
        assert_eq!((e, w), (0, 0));
    }

    #[test]
    fn new_entries_after_mark_seen_become_unseen() {
        let mut log = make_log(&[(Severity::Error, "old")]);
        log.mark_all_seen();
        assert!(!log.has_unseen());

        log.push(Severity::Warning, "new".to_string());
        assert!(log.has_unseen());
        let (e, w) = log.unseen_counts();
        assert_eq!((e, w), (0, 1)); // only the new warning
    }

    #[test]
    fn summary_text_none_when_all_seen() {
        let mut log = make_log(&[(Severity::Error, "e")]);
        log.mark_all_seen();
        assert!(log.summary_text().is_none());
    }

    #[test]
    fn summary_text_errors_only() {
        let log = make_log(&[(Severity::Error, "e1"), (Severity::Error, "e2")]);
        assert_eq!(log.summary_text().unwrap(), "2 errors — :messages for details");
    }

    #[test]
    fn summary_text_single_error() {
        let log = make_log(&[(Severity::Error, "e")]);
        assert_eq!(log.summary_text().unwrap(), "1 error — :messages for details");
    }

    #[test]
    fn summary_text_warnings_only() {
        let log = make_log(&[(Severity::Warning, "w")]);
        assert_eq!(log.summary_text().unwrap(), "1 warning — :messages for details");
    }

    #[test]
    fn summary_text_mixed() {
        let log = make_log(&[
            (Severity::Error, "e"),
            (Severity::Warning, "w1"),
            (Severity::Warning, "w2"),
        ]);
        assert_eq!(
            log.summary_text().unwrap(),
            "1 error, 2 warnings — :messages for details"
        );
    }

    #[test]
    fn summary_text_trace_only() {
        let log = make_log(&[(Severity::Trace, "t1"), (Severity::Trace, "t2")]);
        assert!(log.summary_text().is_none());
    }

    #[test]
    fn format_for_display_empty() {
        let log = MessageLog::new();
        assert_eq!(log.format_for_display(), "");
    }

    #[test]
    fn format_for_display_prefixes() {
        let log = make_log(&[
            (Severity::Warning, "bad key"),
            (Severity::Error,   "crash"),
            (Severity::Trace,   "stack trace here"),
        ]);
        let out = log.format_for_display();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "[warning] bad key");
        assert_eq!(lines[1], "[error] crash");
        assert_eq!(lines[2], "[trace] stack trace here");
    }

    #[test]
    fn scratch_view_cursor_at_last_line() {
        // The scratch view should open with cursor on the last content line.
        let sv = ScratchView::from_text("line1\nline2\nline3\n", "[test]");
        // "line3" starts at char offset 12 (6 + 6).
        let head = sv.sels.primary().head;
        let text = sv.buf.rope().to_string();
        let line_start = text[..text.char_indices()
            .nth(head)
            .map(|(i, _)| i)
            .unwrap_or(0)]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let line_content: String = text[line_start..].chars().take_while(|&c| c != '\n').collect();
        assert_eq!(line_content, "line3");
    }

    #[test]
    fn push_respects_cap() {
        let mut log = MessageLog::new();
        // Push MAX_ENTRIES + 1 entries; the oldest should be evicted.
        for i in 0..=MAX_ENTRIES {
            log.push(Severity::Warning, format!("msg {i}"));
        }
        assert_eq!(log.entries().len(), MAX_ENTRIES);
        // The first entry should now be "msg 1" (msg 0 was evicted).
        let entries: Vec<_> = log.entries().collect();
        assert_eq!(entries[0].text, "msg 1");
        assert_eq!(entries[MAX_ENTRIES - 1].text, format!("msg {MAX_ENTRIES}"));
    }

    #[test]
    fn push_cap_adjusts_seen_up_to() {
        let mut log = MessageLog::new();
        // Push MAX_ENTRIES entries and mark them all seen.
        for i in 0..MAX_ENTRIES {
            log.push(Severity::Warning, format!("msg {i}"));
        }
        log.mark_all_seen();
        assert!(!log.has_unseen());

        // Pushing one more evicts the oldest and shifts seen_up_to.
        log.push(Severity::Error, "overflow".to_string());
        // The new entry is unseen.
        assert!(log.has_unseen());
        let (e, w) = log.unseen_counts();
        assert_eq!((e, w), (1, 0));
    }
}
