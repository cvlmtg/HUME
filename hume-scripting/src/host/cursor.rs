//! Live cursor/selection reads.

use hume_engine::pipeline::BufferId;

/// Live cursor/selection reads — accessed through [`EditorHost::cursor`](super::EditorHost::cursor).
pub trait CursorHost {
    /// Line number (1-indexed) of the primary cursor in the focused buffer.
    ///
    /// Returns `None` when the focused (pane, buffer) has no seeded pane state
    /// (stale or never-focused ids).
    fn current_line_number(&self) -> Option<usize>;

    /// All selections in the focused buffer as `(anchor, head, primary)` triples —
    /// raw 0-indexed char offsets, inclusive model (anchor == head is a 1-char
    /// selection), direction preserved (anchor > head for backward selections),
    /// sorted by selection start, with exactly one triple flagged primary.
    ///
    /// Returns `None` when the focused (pane, buffer) has no seeded pane state.
    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>>;

    /// 1-indexed line number containing the 0-indexed char offset `idx` in the
    /// focused buffer.
    ///
    /// Returns `None` when the focused buffer id is stale (buffer no longer
    /// exists) or when `idx` is out of range (> `len_chars()`).
    fn char_index_to_line(&self, idx: usize) -> Option<usize>;

    /// `(symbol-under-cursor bid)` — the word at the primary cursor head in
    /// the pane currently showing `bid`, `""` on whitespace/punctuation or
    /// when `bid` isn't shown in any pane.
    fn symbol_under_cursor(&self, bid: BufferId) -> String;

    /// `(selections-linewise? bid)` — every *unambiguous* selection in
    /// `bid`'s state, as seen in the pane currently showing it, is linewise
    /// (spans whole lines, anchor to trailing `\n`). A selection collapsed
    /// onto a single empty line is ambiguous (see
    /// `hume_editing::selection::linewise_classification`) and carries no
    /// vote either way. `false` when every selection is ambiguous, matching
    /// an ordinary collapsed cursor's default, and `false` when `bid` isn't
    /// shown in any pane.
    ///
    /// Paired with [`Self::selections_charwise`] to express `:lsp-fmt`'s
    /// three-way verdict (all linewise / none linewise / mixed) as two
    /// booleans rather than a symbol on `lsp-linewise-ranges-params`'s wire
    /// params — every other `lsp-*-params` builtin returns a hash forwarded
    /// to `lsp-request` verbatim or with a *protocol* key inserted, and a
    /// non-protocol verdict key would break that. `(false, false)` from the
    /// pair means *mixed or not-shown*; an all-ambiguous set answers
    /// `(false, true)`, deliberately indistinguishable from all-charwise,
    /// which is the default it's meant to take.
    fn selections_linewise(&self, bid: BufferId) -> bool;

    /// `(selections-charwise? bid)` — no *unambiguous* selection in `bid`'s
    /// state, as seen in the pane currently showing it, is linewise. `true`
    /// when every selection is ambiguous (the complementary default to
    /// [`Self::selections_linewise`]'s `false` in that same case). `false`
    /// when `bid` isn't shown in any pane. See [`Self::selections_linewise`]
    /// for why this is a second predicate rather than a verdict field
    /// elsewhere.
    fn selections_charwise(&self, bid: BufferId) -> bool;
}
