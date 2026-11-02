//! Inlay hints, signs, virtual lines, extra highlights, EOL text,
//! statusline text, and the diagnostic pull/count reads.

use std::sync::Arc;

use hume_core::{BufferId, DiagSeverity, Side};
use hume_rope::line::ContentLine;
use hume_rope::offset::ExclusiveRange;
use hume_rope::position_encoding::PositionEncoding;

use crate::types::VirtualLineSpec;

/// The lines a line-anchored decoration entry marks, never none: `start`
/// alone, or every line up to `end` (`'end-line`, exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkedLines {
    start: ContentLine,
    end: ContentLine,
}

impl MarkedLines {
    pub(crate) fn one(line: ContentLine) -> Self {
        Self {
            start: line,
            end: line.advance(1),
        }
    }

    /// `None` when `end` is not after `start`.
    pub(crate) fn run(start: ContentLine, end: ContentLine) -> Option<Self> {
        (end > start).then_some(Self { start, end })
    }

    pub fn lines(self) -> ExclusiveRange<ContentLine> {
        ExclusiveRange::new(self.start, self.end)
    }
}

/// What `diagnostics-for-buffer`'s `#:range` names: a char range of the
/// buffer as it is, or a request's range, which carries the buffer and text
/// it was taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticRange {
    Chars { start: usize, end: usize },
    Request(crate::builtins::ids::DocRange),
}

/// Why a decoration setter refused its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// The call as a whole: an unknown buffer, an unregistered sign source.
    Call(String),
    /// Entry `index` of the list the call was given.
    Entry { index: usize, message: String },
}

/// The message for a caller with no name for the list: a Steel builtin names
/// the entry by its own argument instead.
impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Call(message) => f.write_str(message),
            Self::Entry { index, message } => write!(f, "entry {index}: {message}"),
        }
    }
}

/// `items` mapped by `entry`, a failure naming the item by its place in
/// `items`.
pub fn each_entry<S, T>(
    items: Vec<S>,
    mut entry: impl FnMut(S) -> Result<T, String>,
) -> Result<Vec<T>, SetError> {
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| entry(item).map_err(|message| SetError::Entry { index, message }))
        .collect()
}

/// Inlay hints, signs, virtual lines, extra highlights, EOL text, statusline
/// text, and the diagnostic pull/count reads, accessed through
/// [`EditorHost::decorations`](super::EditorHost::decorations).
pub trait DecorationHost {
    /// `(set-inlay-hints! pane source hints)`: replaces `source`'s inlay
    /// hints for `bid` wholesale. Each entry is `(offset, text, side)`,
    /// `offset` already a char offset. The Steel builtin accepts char offsets
    /// only (see `lsp-position->char-offset` to convert an LSP wire position).
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, Side)>,
    ) -> Result<(), SetError>;

    /// `(register-sign-source! pane source priority)`: declares `source` a
    /// sign channel at `priority` *for `bid`*, replacing any prior
    /// registration under that source in that buffer (last wins, matching
    /// `register-lsp-server!`). Its gutter slot is its rank among every
    /// source registered for that same buffer, not a property of any one
    /// `set_signs` call, and not shared with any other buffer. A source
    /// claims its slot the first time it becomes relevant to a given
    /// buffer and holds it for that buffer's life, across a reload or a
    /// replaced text, until the buffer closes; there is no withdrawal.
    fn register_sign_source(
        &mut self,
        source: String,
        bid: BufferId,
        priority: i64,
    ) -> Result<(), String>;

    /// `(set-signs! pane source signs)`: replaces `source`'s signs for `bid`
    /// wholesale. Each entry is `(lines, text, scope)`, one sign on each of
    /// `lines`; a line two entries share goes to the first. Every line is
    /// caller-validated against `BufferHost::buffer_line_count`. `Err`,
    /// naming the builtin, if `source` isn't registered for `bid`.
    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(MarkedLines, String, String)>,
    ) -> Result<(), SetError>;

    /// `(set-virtual-lines! pane source lines)`: replaces `source`'s virtual
    /// lines for `bid` wholesale, each entry the line it is anchored to,
    /// caller-validated against `BufferHost::buffer_line_count`, and its
    /// spec.
    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(ContentLine, VirtualLineSpec)>,
    ) -> Result<(), SetError>;

    /// `(set-extra-highlights! pane source spans)`: replaces `source`'s
    /// extra highlights for `bid` wholesale. Each entry is `(start, end,
    /// scope)`, char offsets. `Err` if the range is empty or out of bounds.
    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) -> Result<(), SetError>;

    /// `(set-eol-text! pane source lines)`: replaces `source`'s EOL text for
    /// `bid` wholesale. Each entry is `(line, text, scope)`; `text` is
    /// spliced in at the end of `line`, which is caller-validated against
    /// `BufferHost::buffer_line_count`. `hide_on_insert_line` marks every entry
    /// to be hidden on the primary cursor's line while the pane types in
    /// Insert mode. Not diagnostics-specific: the diagnostics plugin is its
    /// first client, not its owner.
    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(ContentLine, String, String)>,
        hide_on_insert_line: bool,
    ) -> Result<(), SetError>;

    /// `(set-line-backgrounds! pane source entries)`: replaces `source`'s
    /// line backgrounds for `bid` wholesale. Each entry is `(lines, scope)`,
    /// a background on each of `lines`; a line two entries share goes to the
    /// first. Every line is caller-validated against
    /// `BufferHost::buffer_line_count`.
    fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<(MarkedLines, String)>,
    ) -> Result<(), SetError>;

    /// `(set-statusline-text! pane source text)`: replaces `source`'s
    /// statusline text for `bid` wholesale; an empty `text` clears it.
    /// Rendered by the `steel:<source>` statusline element, reading only the
    /// focused buffer's entry. A `bid` that isn't focused simply isn't
    /// shown, not an error. `Err` for an unknown `bid`.
    fn set_statusline_text(
        &mut self,
        source: String,
        bid: BufferId,
        text: String,
    ) -> Result<(), String>;

    /// `(diagnostics-for-buffer pane #:severity floor #:range (hash 'start-char s 'end-char e))`:
    /// one [`DiagnosticEntry`] per diagnostic, filtered then capped at 1000.
    /// A read the cap cuts is traced in `:messages`.
    /// `start_char` is an addressing unit (feeds `goto-location!`);
    /// `grapheme_col` is the display unit (the one every HUME surface shows
    /// the user). `severity_floor` is `None` for the
    /// `lsp-diagnostics-min-severity` option's floor; `range` is `None` for
    /// the whole buffer.
    fn diagnostics_for_buffer(
        &mut self,
        bid: BufferId,
        severity_floor: Option<DiagSeverity>,
        range: Option<DiagnosticRange>,
    ) -> Result<Vec<DiagnosticEntry>, String>;

    /// `(diagnostic-counts pane #:severity floor)`: `bid`'s diagnostics
    /// counted per severity, those below `floor` (default
    /// `lsp-diagnostics-min-severity`) as none.
    fn diagnostic_counts(&self, bid: BufferId, floor: Option<DiagSeverity>) -> DiagnosticCounts;
}

/// How many of a buffer's diagnostics carry each severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticCounts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
    pub hint: usize,
}

impl DiagnosticCounts {
    /// The count for `severity`.
    pub fn get(mut self, severity: DiagSeverity) -> usize {
        *self.of(severity)
    }

    /// The count for `severity`, to add to.
    pub fn of(&mut self, severity: DiagSeverity) -> &mut usize {
        match severity {
            DiagSeverity::Error => &mut self.error,
            DiagSeverity::Warning => &mut self.warning,
            DiagSeverity::Info => &mut self.info,
            DiagSeverity::Hint => &mut self.hint,
        }
    }

    /// The count over every severity.
    pub fn total(self) -> usize {
        self.error + self.warning + self.info + self.hint
    }
}

/// One `diagnostics-for-buffer` result entry; see
/// [`DecorationHost::diagnostics_for_buffer`]. A typed struct rather than a
/// `serde_json::Value` blob because every field but `raw` is HUME-computed
/// (native Steel data the builtin decodes field-by-field), while `raw` is
/// the server's own wire `Diagnostic`: the one field that crosses as an
/// opaque `JsonHandle` sharing the same `Arc` the diagnostics store already
/// holds, instead of a value rebuilt (and reconverted) just to carry it.
#[derive(Debug, Clone)]
pub struct DiagnosticEntry {
    pub start_char: usize,
    pub end_char: usize,
    pub start_line: ContentLine,
    pub end_line: ContentLine,
    pub grapheme_col: usize,
    pub severity: DiagSeverity,
    pub message: String,
    pub code: Option<String>,
    pub source: Option<String>,
    pub raw: Arc<serde_json::Value>,
    /// The publishing server and its negotiated encoding at ingest time.
    /// They tag `raw`'s `JsonHandle` so a wire position inside it decodes
    /// correctly even after that server has restarted or detached.
    pub server: crate::ServerRef,
    pub encoding: PositionEncoding,
}
