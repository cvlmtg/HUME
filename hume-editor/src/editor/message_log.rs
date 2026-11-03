//! Persistent message log.
//!
//! The [`MessageLog`] accumulates [`LogEntry`] values produced during an editing
//! session: config warnings, scripting errors, plugin conflicts. Entries survive
//! keypresses and can be reviewed at any time via `:messages`.

use std::collections::VecDeque;

use hume_core::{BufferId, Severity};
use hume_editing::text::normalize_line_endings_owned;
use hume_engine::theme::diagnostic_scopes;
use hume_scripting::host::DecorationHost;

use super::buffer::{EditorView, ViewLabel};
use super::error::CommandError;
use super::host_impl::EditorHostImpl;
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
/// A push into a full log searches it for the entry to evict and stops at
/// the oldest trace, which a trace flood keeps near the front. Only a log of
/// nearly all errors pays the O(`MAX_ENTRIES`) worst case.
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
    /// on the status line. Both ask what was reported, whether or not
    /// `messages-min-severity` shows it.
    total_errors: u64,
    total_warnings: u64,
    /// Entries pushed plus entries evicted at each [`MessagesMinSeverity`]
    /// level, indexed by the level. What a floor shows changes only when a
    /// level it shows moves.
    changes: [u64; 3],
}

/// What the `[messages]` buffer was last built from: a floor and the
/// changes so far at the levels it shows. A different stamp means the
/// buffer is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct MessagesViewStamp {
    floor: MessagesMinSeverity,
    shown_changes: u64,
}

impl MessageLog {
    pub(in crate::editor) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            seen_up_to: 0,
            total_errors: 0,
            total_warnings: 0,
            changes: [0; 3],
        }
    }

    /// Append an entry to the log, whatever severity `messages-min-severity`
    /// shows.
    ///
    /// When the entry count would exceed [`MAX_ENTRIES`], the oldest trace is
    /// evicted, else the oldest warning, else the oldest error, and
    /// `seen_up_to` moves back when the evicted entry was a seen one.
    ///
    /// The text is stored with LF line endings, as a buffer holds it, so
    /// [`Self::format_with_spans`]' spans fit the `[messages]` text.
    pub(in crate::editor) fn push(&mut self, severity: Severity, text: String) {
        self.changes[MessagesMinSeverity::of(severity) as usize] += 1;
        match severity {
            Severity::Error => self.total_errors += 1,
            Severity::Warning => self.total_warnings += 1,
            Severity::Info | Severity::Trace => {}
        }
        if self.entries.len() == MAX_ENTRIES {
            let index = self.eviction_index();
            if let Some(evicted) = self.entries.remove(index) {
                self.changes[MessagesMinSeverity::of(evicted.severity) as usize] += 1;
            }
            if index < self.seen_up_to {
                self.seen_up_to -= 1;
            }
        }
        self.entries.push_back(LogEntry {
            severity,
            text: normalize_line_endings_owned(text),
        });
    }

    /// The stamp of the log as `floor` shows it now.
    pub(in crate::editor) fn view_stamp(&self, floor: MessagesMinSeverity) -> MessagesViewStamp {
        MessagesViewStamp {
            floor,
            shown_changes: self.changes[floor as usize..].iter().sum(),
        }
    }

    /// Whether `floor` shows any entry.
    pub(in crate::editor) fn shows_any(&self, floor: MessagesMinSeverity) -> bool {
        self.shown_from(0, floor).next().is_some()
    }

    /// The oldest entry of the least severe kind present. A log with no
    /// trace and no warning holds only errors, so its oldest is the first.
    fn eviction_index(&self) -> usize {
        let oldest_at = |level| {
            self.entries
                .iter()
                .position(|entry| MessagesMinSeverity::of(entry.severity) == level)
        };
        oldest_at(MessagesMinSeverity::Trace)
            .or_else(|| oldest_at(MessagesMinSeverity::Warning))
            .unwrap_or(0)
    }

    /// The entries from index `start` on that `floor` shows.
    pub(in crate::editor::message_log) fn shown_from(
        &self,
        start: usize,
        floor: MessagesMinSeverity,
    ) -> impl Iterator<Item = &LogEntry> {
        self.entries
            .iter()
            .skip(start)
            .filter(move |entry| floor.shows(entry.severity))
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

    /// Count of unseen entries `floor` shows, by severity: `(errors,
    /// warnings)`.
    ///
    /// `Info` entries are never logged; `Trace` entries are not surfaced in
    /// the summary because they are supplemental detail, not actionable items.
    pub(in crate::editor::message_log) fn unseen_counts(
        &self,
        floor: MessagesMinSeverity,
    ) -> (usize, usize) {
        self.shown_from(self.seen_up_to, floor)
            .fold((0, 0), |(e, w), entry| match entry.severity {
                Severity::Error => (e + 1, w),
                Severity::Warning => (e, w + 1),
                _ => (e, w),
            })
    }

    /// Mark all current entries as seen.
    ///
    /// Called when the user opens `:messages`, on every frame a pane on the
    /// active tab shows `[messages]`, and after the statusline summary's
    /// keystroke budget elapses.
    pub(in crate::editor) fn mark_all_seen(&mut self) {
        self.seen_up_to = self.entries.len();
    }

    /// One-line summary shown in the statusline when there are unseen
    /// `Warning` or `Error` entries.
    ///
    /// Returns `None` when everything has been seen, *or* when no unseen
    /// entry `floor` shows is a `Warning` or `Error`: trace messages never
    /// raise a statusline indicator (see [`Severity`] table).
    pub(crate) fn summary_text(&self, floor: MessagesMinSeverity) -> Option<String> {
        if !self.has_unseen() {
            return None;
        }
        let (errors, warnings) = self.unseen_counts(floor);
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

    /// The entries `floor` shows, formatted for display in the `:messages`
    /// scratch buffer, along with the severity-highlight spans for each line.
    ///
    /// Each line is `[severity] text\n`. Returns an empty string and no
    /// spans if no entry is shown. Spans are `(start, end, scope name)`
    /// char offsets (not bytes), since message text may be non-ASCII. Scope names,
    /// not `ScopeId`s: this type has no registry access (it's not an
    /// `Editor` method), so interning is the caller's job. See
    /// [`Editor::sync_messages_view`], the sole production caller, which
    /// turns each name into an `ExtraHighlightEntry`.
    ///
    /// Highlighting goes through the `Extra` tier rather than tree-sitter:
    /// `[messages]` is a synthetic, path-less buffer, so `detect_language`
    /// never attaches a grammar to it (and a grammar for `[severity] text`
    /// lines would be absurd anyway).
    pub(in crate::editor) fn format_with_spans(
        &self,
        floor: MessagesMinSeverity,
    ) -> (String, Vec<(usize, usize, &'static str)>) {
        let mut out = String::new();
        let mut spans = Vec::new();
        let mut pos = 0;
        for entry in self.shown_from(0, floor) {
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
    /// only care about the rendered text of every stored entry.
    #[cfg(test)]
    pub(in crate::editor) fn format_for_display(&self) -> String {
        self.format_with_spans(MessagesMinSeverity::Trace).0
    }
}

impl Editor {
    /// Bring `[messages]` up to date with the log and `messages-min-severity`
    /// when either changed since it was last built, and mark the log seen
    /// while a pane on the active tab shows it. Runs once per frame.
    pub(in crate::editor) fn follow_message_log(&mut self) {
        let label = ViewLabel::editor(EditorView::Messages);
        let Some(bid) = self.state.buffers.find_by_label(&label) else {
            return;
        };
        let floor = self.state.settings.messages_min_severity;
        if self.state.config.messages_view != Some(self.state.message_log.view_stamp(floor)) {
            self.sync_messages_view(bid);
        }
        if self
            .view
            .active_pane_ids()
            .into_iter()
            .any(|pid| self.view.panes[pid].buffer_id() == bid)
        {
            self.state.message_log.mark_all_seen();
        }
    }

    /// Rebuild `[messages]` (`bid`) from the log at the current
    /// `messages-min-severity`, with its severity highlights. Every stored
    /// position is carried through the change, so no cursor moves on its
    /// own.
    pub(in crate::editor) fn sync_messages_view(&mut self, bid: BufferId) {
        let floor = self.state.settings.messages_min_severity;
        let (content, spans) = self.state.message_log.format_with_spans(floor);
        let (buffers, mut stores) = self.state.edit_parts();
        buffers
            .get_mut(bid)
            .follow_view_content(bid, &mut stores, &content);
        let spans = spans
            .into_iter()
            .map(|(start, end, scope)| (start, end, scope.to_string()))
            .collect();
        EditorHostImpl::new(&mut self.state, &mut self.view)
            .set_extra_highlights("messages".to_string(), bid, spans)
            .expect("the spans lie in the text they were formatted with");
        self.state.config.messages_view = Some(self.state.message_log.view_stamp(floor));
    }

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
