//! Inlay hints, signs, virtual lines, extra highlights, EOL text,
//! statusline text, and the diagnostic pull/count reads.

use hume_engine::pipeline::BufferId;

use crate::types::VirtualLineSpec;

/// Inlay hints, signs, virtual lines, extra highlights, EOL text, statusline
/// text, and the diagnostic pull/count reads — accessed through
/// [`EditorHost::decorations`](super::EditorHost::decorations).
pub trait DecorationHost {
    /// `(set-inlay-hints! source bid hints)` — replaces `source`'s inlay
    /// hints for `bid` wholesale. Each entry is `(offset, text, before)`,
    /// `offset` already a char offset — the Steel builtin no longer accepts
    /// LSP wire positions directly (see `lsp-position->offset`).
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, bool)>,
    ) -> Result<(), String>;

    /// `(register-sign-source! name bid priority)` — declares `name` a sign
    /// channel at `priority` *for `bid`*, replacing any prior registration
    /// under that name in that buffer (last wins, matching
    /// `register-lsp-server!`). Its gutter slot is its rank among every
    /// source registered for that same buffer, not a property of any one
    /// `set_signs` call, and not shared with any other buffer — a source
    /// claims its slot the first time it becomes relevant to a given
    /// buffer and holds it for that buffer's life; there is no withdrawal.
    fn register_sign_source(
        &mut self,
        name: String,
        bid: BufferId,
        priority: i64,
    ) -> Result<(), String>;

    /// `(set-signs! source bid signs)` — replaces `source`'s signs for `bid`
    /// wholesale. Each entry is `(line, text, scope)`; `line` converts to
    /// that line's line-start char offset at this boundary — `Err`, naming
    /// the builtin, if `line` is out of range or `source` isn't registered
    /// for `bid`.
    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String)>,
    ) -> Result<(), String>;

    /// `(set-virtual-lines! source bid lines)` — replaces `source`'s virtual
    /// lines for `bid` wholesale. Each `VirtualLineSpec`'s `segments` are
    /// **unvalidated** char ranges (the Steel boundary only decodes shape,
    /// see `VirtualLineSpec`'s doc) — this method is the sole enforcement
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

    /// `(set-extra-highlights! source bid spans)` — replaces `source`'s
    /// extra highlights for `bid` wholesale. Each entry is `(start, end,
    /// scope)`, char offsets — `Err`, naming the builtin, if the range is
    /// empty or out of bounds.
    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) -> Result<(), String>;

    /// `(set-eol-text! source bid lines)` — replaces `source`'s EOL text for
    /// `bid` wholesale. Each entry is `(line, text, scope)`; `text` is
    /// spliced in at the end of `line`, which converts to that line's
    /// line-start char offset at this boundary — `Err`, naming the builtin,
    /// if `line` is out of range. Not diagnostics-specific — the diagnostics
    /// plugin is its first client, not its owner.
    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, String)>,
    ) -> Result<(), String>;

    /// `(set-line-backgrounds! source bid entries)` — replaces `source`'s
    /// line backgrounds for `bid` wholesale. Each entry is `(line, scope)`;
    /// `line` converts to that line's line-start char offset at this
    /// boundary — `Err`, naming the builtin, if `line` is out of range.
    fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<(usize, String)>,
    ) -> Result<(), String>;

    /// `(set-statusline-text! source bid text)` — replaces `source`'s
    /// statusline text for `bid` wholesale; an empty `text` clears it.
    /// Rendered by the `steel:<source>` statusline element, reading only the
    /// focused buffer's entry — a `bid` that isn't focused simply isn't
    /// shown, not an error. `Err` for an unknown `bid`.
    fn set_statusline_text(
        &mut self,
        source: String,
        bid: BufferId,
        text: String,
    ) -> Result<(), String>;

    /// `(diagnostics-for-buffer bid #:severity floor #:range (start end))` —
    /// decoded `{"start" "end" "line" "char-col" "grapheme-col" "severity"
    /// "message" "code" "source"}` hashmaps, filtered then capped at 1000.
    /// `char-col` is an addressing unit (feeds `goto-location!`);
    /// `grapheme-col` is the display unit (the one every HUME surface shows
    /// the user) — never render `char-col` directly. `severity_floor`
    /// is `None` for "no floor" (everything); `range` is `None` for the
    /// whole buffer. `Err` on an unknown `#:severity` name.
    fn diagnostics_for_buffer(
        &self,
        bid: BufferId,
        severity_floor: Option<&str>,
        range: Option<(usize, usize)>,
    ) -> Result<Vec<serde_json::Value>, String>;

    /// `(diagnostic-counts bid)` → `(errors . warnings)`.
    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize);
}
