//! Inlay hints, signs, virtual lines, extra highlights, EOL text,
//! statusline text, and the diagnostic pull/count reads.

use std::sync::Arc;

use hume_engine::pipeline::BufferId;
use hume_rope::position_encoding::PositionEncoding;

use crate::types::VirtualLineSpec;

/// Inlay hints, signs, virtual lines, extra highlights, EOL text, statusline
/// text, and the diagnostic pull/count reads, accessed through
/// [`EditorHost::decorations`](super::EditorHost::decorations).
pub trait DecorationHost {
    /// `(set-inlay-hints! source pane hints)`: replaces `source`'s inlay
    /// hints for `bid` wholesale. Each entry is `(offset, text, before)`,
    /// `offset` already a char offset. The Steel builtin accepts char offsets
    /// only (see `lsp-position->offset` to convert an LSP wire position).
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, bool)>,
    ) -> Result<(), String>;

    /// `(register-sign-source! name pane priority)`: declares `name` a sign
    /// channel at `priority` *for `bid`*, replacing any prior registration
    /// under that name in that buffer (last wins, matching
    /// `register-lsp-server!`). Its gutter slot is its rank among every
    /// source registered for that same buffer, not a property of any one
    /// `set_signs` call, and not shared with any other buffer. A source
    /// claims its slot the first time it becomes relevant to a given
    /// buffer and holds it for that buffer's life; there is no withdrawal.
    fn register_sign_source(
        &mut self,
        name: String,
        bid: BufferId,
        priority: i64,
    ) -> Result<(), String>;

    /// `(set-signs! source pane signs)`: replaces `source`'s signs for `bid`
    /// wholesale. Each entry is `(line, text, scope)`; `line` converts to
    /// that line's line-start char offset at this boundary. `Err`, naming
    /// the builtin, if `line` is out of range or `source` isn't registered
    /// for `bid`.
    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String)>,
    ) -> Result<(), String>;

    /// `(set-virtual-lines! source pane lines)`: replaces `source`'s virtual
    /// lines for `bid` wholesale. Each `VirtualLineSpec`'s `segments` are
    /// **unvalidated** char ranges (the Steel boundary only decodes shape,
    /// see `VirtualLineSpec`'s doc), so this method is the sole enforcement
    /// point: it must sort, validate (bounds, ordering, non-overlap,
    /// grapheme-cluster alignment against `text`), and convert to byte
    /// offsets, `Err`ing with a message naming `set-virtual-lines!` on any
    /// violation rather than storing bad data.
    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<VirtualLineSpec>,
    ) -> Result<(), String>;

    /// `(set-extra-highlights! source pane spans)`: replaces `source`'s
    /// extra highlights for `bid` wholesale. Each entry is `(start, end,
    /// scope)`, char offsets. `Err`, naming the builtin, if the range is
    /// empty or out of bounds.
    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) -> Result<(), String>;

    /// `(set-eol-text! source pane lines)`: replaces `source`'s EOL text for
    /// `bid` wholesale. Each entry is `(line, text, scope)`; `text` is
    /// spliced in at the end of `line`, which converts to that line's
    /// line-start char offset at this boundary. `Err`, naming the builtin,
    /// if `line` is out of range. Not diagnostics-specific: the diagnostics
    /// plugin is its first client, not its owner.
    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, String)>,
    ) -> Result<(), String>;

    /// `(set-line-backgrounds! source pane entries)`: replaces `source`'s
    /// line backgrounds for `bid` wholesale. Each entry is `(line, scope)`;
    /// `line` converts to that line's line-start char offset at this
    /// boundary. `Err`, naming the builtin, if `line` is out of range.
    fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<(usize, String)>,
    ) -> Result<(), String>;

    /// `(set-statusline-text! source pane text)`: replaces `source`'s
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

    /// `(diagnostics-for-buffer pane #:severity floor #:range (start end))`:
    /// one [`DiagnosticEntry`] per diagnostic, filtered then capped at 1000.
    /// `char_col` is an addressing unit (feeds `goto-location!`);
    /// `grapheme_col` is the display unit (the one every HUME surface shows
    /// the user). Never render `char_col` directly. `severity_floor`
    /// is `None` for "no floor" (everything); `range` is `None` for the
    /// whole buffer. `Err` on an unknown `#:severity` name.
    fn diagnostics_for_buffer(
        &self,
        bid: BufferId,
        severity_floor: Option<&str>,
        range: Option<(usize, usize)>,
    ) -> Result<Vec<DiagnosticEntry>, String>;

    /// `(diagnostic-counts pane)` → `(errors . warnings)`.
    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize);
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
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub end_line: usize,
    pub char_col: usize,
    pub grapheme_col: usize,
    pub severity: String,
    /// `DiagSeverity`'s own `Ord` discriminant (0 = error … 3 = hint, lower
    /// is more severe): the single encoding of severity order, so Scheme
    /// compares by this instead of re-deriving the same ranking from
    /// `severity`'s string form.
    pub severity_rank: u8,
    pub message: String,
    pub code: Option<String>,
    pub source: Option<String>,
    pub raw: Arc<serde_json::Value>,
    /// The publishing server's negotiated encoding at ingest time. Tags
    /// `raw`'s `JsonHandle` so a wire position inside it decodes correctly
    /// even after the server that sent it has since restarted or detached.
    pub encoding: PositionEncoding,
}
