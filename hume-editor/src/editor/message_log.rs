//! Persistent message log.
//!
//! The [`MessageLog`] accumulates [`LogEntry`] values produced during an editing
//! session: config warnings, scripting errors, plugin conflicts. Entries survive
//! keypresses and can be reviewed at any time via `:messages`.

use std::collections::VecDeque;

use hume_core::{BufferId, Severity};
use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::apply_keeping_final_break;
use hume_editing::text::{BufferText, normalize_line_endings_owned};
use hume_engine::theme::diagnostic_scopes;
use hume_rope::offset::CharOffset;

use super::buffer::{EditorView, ViewLabel, lifecycle};
use super::error::CommandError;
use super::host_impl::EditorHostImpl;
use super::settings::{MessagesMinSeverity, SettingText};
use super::{Editor, EditorState};

/// A single entry in the persistent message log.
#[derive(Debug, Clone)]
pub(in crate::editor) struct LogEntry {
    pub(in crate::editor) severity: Severity,
    pub(in crate::editor) text: String,
    /// How many entries were pushed before this one. Entries stay in `seq`
    /// order: eviction removes but never reorders.
    seq: u64,
}

/// A `[messages]` highlight: `(start, end, scope name)` in chars.
type MessageSpan = (usize, usize, &'static str);

/// The extra-highlight source of the `[messages]` highlights.
const HIGHLIGHT_SOURCE: &str = "messages";

/// The entries a `[messages]` text shows, oldest first, each as its `seq`
/// and the char length of its line.
type Layout = Vec<(u64, usize)>;

/// What the `[messages]` text was last built from, and what it holds.
#[derive(Debug)]
pub(in crate::editor) struct MessagesView {
    stamp: MessagesViewStamp,
    layout: Layout,
}

/// [`MessageLog::follow_view`]'s result.
struct ViewFollow {
    /// What the view's floor shows now.
    text: BufferText,
    /// The change from the view's text to `text`.
    change: ChangeSet,
    /// `text`'s layout.
    layout: Layout,
    /// The inserted entries' highlights, in `text`.
    spans: Vec<MessageSpan>,
}

// ── MessageLog ───────────────────────────────────────────────────────────────

/// Maximum number of entries kept in the log.
///
/// A push into a full log searches it for the entry to evict and stops at
/// the oldest trace, which a trace flood keeps near the front. A full log
/// with no trace entry scans every entry before it evicts.
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
    /// Lifetime count of messages ever pushed at each [`MessagesMinSeverity`]
    /// level, indexed by the level: monotonic, never decremented by eviction
    /// (unlike `unseen_counts`, which reads the live `entries` deque and so
    /// *can* drop below a value it reported earlier once `MAX_ENTRIES` starts
    /// evicting). `:reload-config` diffs the error and warning counts against
    /// a before/after snapshot to detect "did this reload push a new warning
    /// or error", and `handle_input` to keep a command's message on the
    /// status line. Both ask what was reported, whether or not
    /// `messages-min-severity` shows it.
    pushed: [u64; LEVELS],
    /// Entries evicted at each level, indexed like `pushed`. What a floor
    /// shows changes only when a level it shows gains a push or an eviction.
    evicted: [u64; LEVELS],
}

/// How many [`MessagesMinSeverity`] levels there are.
const LEVELS: usize = <MessagesMinSeverity as SettingText>::VALUES.len();

/// What the `[messages]` buffer was last built from: a floor and the
/// changes so far at the levels it shows. A different stamp means the
/// buffer is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MessagesViewStamp {
    floor: MessagesMinSeverity,
    shown_changes: u64,
}

impl MessageLog {
    pub(in crate::editor) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            seen_up_to: 0,
            pushed: [0; LEVELS],
            evicted: [0; LEVELS],
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
    /// [`Self::format_view`]'s spans fit the `[messages]` text.
    pub(in crate::editor) fn push(&mut self, severity: Severity, text: String) {
        let seq = self.pushed.iter().sum();
        self.pushed[MessagesMinSeverity::of(severity) as usize] += 1;
        if self.entries.len() == MAX_ENTRIES {
            let index = self.eviction_index();
            if let Some(evicted) = self.entries.remove(index) {
                self.evicted[MessagesMinSeverity::of(evicted.severity) as usize] += 1;
            }
            if index < self.seen_up_to {
                self.seen_up_to -= 1;
            }
        }
        self.entries.push_back(LogEntry {
            severity,
            text: normalize_line_endings_owned(text),
            seq,
        });
    }

    /// The stamp of the log as `floor` shows it now.
    fn view_stamp(&self, floor: MessagesMinSeverity) -> MessagesViewStamp {
        MessagesViewStamp {
            floor,
            shown_changes: self.pushed[floor as usize..]
                .iter()
                .chain(&self.evicted[floor as usize..])
                .sum(),
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
        (
            self.pushed[MessagesMinSeverity::Error as usize],
            self.pushed[MessagesMinSeverity::Warning as usize],
        )
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
    /// Called on every frame that draws the end of an up-to-date
    /// `[messages]`, and after the statusline summary's keystroke budget
    /// elapses.
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
    /// scratch buffer, along with the severity-highlight spans for each line
    /// and the text's layout.
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
    fn format_view(&self, floor: MessagesMinSeverity) -> (String, Vec<MessageSpan>, Layout) {
        let mut out = String::new();
        let mut spans = Vec::new();
        let mut layout = Vec::new();
        let mut pos = 0;
        for entry in self.shown_from(0, floor) {
            let chars = format_entry(entry, pos, &mut out, &mut spans);
            layout.push((entry.seq, chars));
            pos += chars;
        }
        (out, spans, layout)
    }

    /// [`Self::format_view`]'s text and spans.
    #[cfg(test)]
    pub(in crate::editor) fn format_with_spans(
        &self,
        floor: MessagesMinSeverity,
    ) -> (String, Vec<MessageSpan>) {
        let (text, spans, _) = self.format_view(floor);
        (text, spans)
    }

    /// The change that takes the `[messages]` text `text`, laid out as
    /// `layout`, to [`Self::format_view`]'s for `floor`. An entry the log
    /// evicted or `floor` hides is deleted, an entry new to the view is
    /// formatted and inserted in log order, and every other line is kept.
    fn follow_view(
        &self,
        layout: &[(u64, usize)],
        floor: MessagesMinSeverity,
        text: &BufferText,
    ) -> ViewFollow {
        let end = text.end();
        debug_assert_eq!(
            end.index(),
            layout.iter().map(|&(_, chars)| chars).sum::<usize>().max(1),
            "follow_view: the layout must describe the text"
        );
        let (text, change, (layout, spans)) = apply_keeping_final_break(text, |ceiling| {
            let stop = |to: usize| ceiling.map_or(to, |c| to.min(c.index()));
            let mut builder = ChangeSetBuilder::new(end);
            let mut new_layout = Vec::with_capacity(layout.len());
            let mut spans = Vec::new();
            let mut line = String::new();
            let (mut old, mut new) = (0, 0);
            let mut viewed = layout.iter().peekable();
            for entry in self.shown_from(0, floor) {
                while let Some(&(_, chars)) = viewed.next_if(|&&(seq, _)| seq < entry.seq) {
                    builder
                        .retain_to(CharOffset::new(old))
                        .delete_to(CharOffset::new(stop(old + chars)));
                    old += chars;
                }
                if let Some(&(seq, chars)) = viewed.next_if(|&&(seq, _)| seq == entry.seq) {
                    new_layout.push((seq, chars));
                    old += chars;
                    new += chars;
                } else {
                    let chars = format_entry(entry, new, &mut line, &mut spans);
                    builder.retain_to(CharOffset::new(old)).insert(&line);
                    line.clear();
                    new_layout.push((entry.seq, chars));
                    new += chars;
                }
            }
            let rest_end = stop(end.index());
            if old < rest_end {
                builder
                    .retain_to(CharOffset::new(old))
                    .delete_to(CharOffset::new(rest_end));
            }
            (builder.finish(), (new_layout, spans))
        });
        ViewFollow {
            text,
            change,
            layout,
            spans,
        }
    }

    /// Test-only convenience over [`Self::format_view`] for assertions that
    /// only care about the rendered text of every stored entry.
    #[cfg(test)]
    pub(in crate::editor) fn format_for_display(&self) -> String {
        self.format_view(MessagesMinSeverity::Trace).0
    }
}

/// Append `entry`'s line, `[severity] text\n`, to `out`, and its badge and
/// text highlights to `spans`, the line starting at char offset `start`: no
/// text span for an empty text. Returns the line's length in chars.
fn format_entry(
    entry: &LogEntry,
    start: usize,
    out: &mut String,
    spans: &mut Vec<MessageSpan>,
) -> usize {
    out.push('[');
    out.push_str(entry.severity.name());
    out.push_str("] ");
    out.push_str(&entry.text);
    out.push('\n');
    let badge_end = start + entry.severity.name().len() + 2; // '[' + label + ']'
    spans.push((
        start,
        badge_end,
        diagnostic_scopes::message_badge(entry.severity),
    ));
    let text_start = badge_end + 1;
    let text_end = text_start + entry.text.chars().count();
    if text_end > text_start {
        spans.push((
            text_start,
            text_end,
            diagnostic_scopes::message_text(entry.severity),
        ));
    }
    text_end + 1 - start
}

impl Editor {
    /// Bring `[messages]` up to date when [`Self::messages_view_behind`]. A
    /// hidden view catches up when it is next shown.
    pub(in crate::editor) fn follow_message_log(&mut self) {
        if self.messages_view_behind() {
            self.sync_messages_view();
        }
    }

    /// Whether the log or `messages-min-severity` changed since `[messages]`
    /// was last built and a pane on the active tab shows it.
    pub(in crate::editor) fn messages_view_behind(&self) -> bool {
        if !self.state.messages_view_stale() {
            return false;
        }
        let label = ViewLabel::editor(EditorView::Messages);
        let Some(bid) = self.state.buffers.find_by_label(&label) else {
            return false;
        };
        self.view
            .active_pane_ids()
            .into_iter()
            .any(|pid| self.view.panes[pid].buffer_id() == bid)
    }

    /// The `[messages]` buffer, opened from the log or brought up to date
    /// with it at the current `messages-min-severity` when it is missing or
    /// stale, with its severity highlights. Only the entries that changed
    /// are formatted, except after a config reload drops the highlights,
    /// when every entry's are rebuilt. The change carries every stored
    /// position, so no cursor moves on its own.
    pub(in crate::editor) fn sync_messages_view(&mut self) -> BufferId {
        let label = ViewLabel::editor(EditorView::Messages);
        let existing = self.state.buffers.find_by_label(&label);
        if let Some(bid) = existing
            && !self.state.messages_view_stale()
        {
            return bid;
        }
        let floor = self.state.settings.messages_min_severity;
        let (bid, layout, spans) = match existing {
            Some(bid) => {
                let view = self
                    .state
                    .messages_view
                    .take()
                    .expect("an open `[messages]` was built from the log");
                let follow = self.state.message_log.follow_view(
                    &view.layout,
                    floor,
                    self.state.buffers.get(bid).text(),
                );
                let (buffers, mut stores) = self.state.edit_parts();
                buffers.get_mut(bid).follow_view_change(
                    bid,
                    &mut stores,
                    follow.text,
                    &follow.change,
                );
                let spans = if self.state.config.messages_highlighted {
                    follow.spans
                } else {
                    // A fresh `ConfigState` holds none of the view's highlights.
                    self.state.message_log.format_view(floor).1
                };
                (bid, follow.layout, spans)
            }
            None => {
                let (content, spans, layout) = self.state.message_log.format_view(floor);
                let bid = lifecycle::open_view_buffer(
                    &mut self.state,
                    &mut self.view,
                    BufferText::from(content.as_str()),
                    &label,
                );
                (bid, layout, spans)
            }
        };
        EditorHostImpl::new(&mut self.state, &mut self.view)
            .add_extra_highlights(HIGHLIGHT_SOURCE.to_string(), bid, spans)
            .expect("the spans lie in the text they were formatted for");
        self.state.config.messages_highlighted = true;
        self.state.messages_view = Some(MessagesView {
            stamp: self.state.message_log.view_stamp(floor),
            layout,
        });
        bid
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
    /// Whether the log or `messages-min-severity` changed since `[messages]`
    /// was last built, it never was, or its highlights are missing.
    pub(in crate::editor) fn messages_view_stale(&self) -> bool {
        !self.config.messages_highlighted
            || self.messages_view.as_ref().map(|view| view.stamp)
                != Some(
                    self.message_log
                        .view_stamp(self.settings.messages_min_severity),
                )
    }

    /// Whether `bid` is an up-to-date `[messages]` and the log holds unseen
    /// entries, so a frame drawing its end marks them seen. A stale view
    /// lacks what was logged since it was built, so its end is not the
    /// log's end.
    pub(in crate::editor) fn messages_view_awaits_seen(&self, bid: BufferId) -> bool {
        self.message_log.has_unseen()
            && self.buffers.get(bid).label() == Some(&ViewLabel::editor(EditorView::Messages))
            && !self.messages_view_stale()
    }

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
