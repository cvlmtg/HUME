//! Live cursor/selection reads.

use hume_engine::pipeline::BufferId;

use crate::types::PaneHandle;

/// Live cursor/selection reads, accessed through [`EditorHost::cursor`](super::EditorHost::cursor).
///
/// Every method but [`Self::offset_to_line`] is kind-B (see `docs/LSP.md`'s
/// pane-targeting convention): it acts through `pane`'s own pane, and raises
/// (`Err`) rather than answering a default when `pane` carries no pane, a
/// closed one, or one that no longer shows `pane`'s buffer: the same
/// fail-fast contract every pane-needing builtin shares.
pub trait CursorHost {
    /// `(buffer-cursor-line pane)`: 0-indexed line of the primary
    /// cursor in `pane`'s own pane.
    fn buffer_cursor_line(&self, pane: PaneHandle) -> Result<usize, String>;

    /// `(buffer-selections pane)`: every selection in `pane`'s own pane, as
    /// `(anchor, head, primary)` triples of raw 0-indexed char offsets,
    /// inclusive model (anchor == head is a 1-char selection), direction
    /// preserved (anchor > head for backward selections), sorted by
    /// selection start, with exactly one triple flagged primary.
    fn buffer_selections(&self, pane: PaneHandle) -> Result<Vec<(usize, usize, bool)>, String>;

    /// `(offset->line bid idx)`: 0-indexed line containing the
    /// 0-indexed char offset `idx` in `bid`'s live text. Pure text math, not
    /// selection/pane state. Kind-C, unlike every other method here: `bid`
    /// need not be shown in any pane.
    ///
    /// Returns `None` when `idx` is out of range (> `len_chars()`). `bid`'s
    /// liveness is checked at argument-resolve time (`args::LivePane`), so a
    /// stale `bid` never reaches here.
    fn offset_to_line(&self, bid: BufferId, idx: usize) -> Option<usize>;

    /// `(symbol-under-cursor pane)`: the word at the primary cursor head in
    /// `pane`'s own pane, `""` on whitespace/punctuation.
    fn symbol_under_cursor(&self, pane: PaneHandle) -> Result<String, String>;

    /// `(selections-linewise? pane)`: every *unambiguous* selection in
    /// `pane`'s own pane is linewise (spans whole lines, anchor to trailing
    /// `\n`). A selection collapsed onto a single empty line is ambiguous
    /// (see `hume_editing::selection::linewise_classification`) and carries
    /// no vote either way. `false` when every selection is ambiguous,
    /// matching an ordinary collapsed cursor's default.
    ///
    /// Paired with [`Self::selections_charwise`] to express `:lsp-fmt`'s
    /// three-way verdict (all linewise / none linewise / mixed) as two
    /// booleans rather than a symbol on `lsp-linewise-ranges-params`'s wire
    /// params. Every other `lsp-*-params` builtin returns a hash forwarded
    /// to `lsp-request!` verbatim or with a *protocol* key inserted, and a
    /// non-protocol verdict key would break that. `(false, false)` from the
    /// pair means *mixed*; an all-ambiguous set answers `(false, true)`,
    /// deliberately indistinguishable from all-charwise, which is the
    /// default it's meant to take.
    fn selections_linewise(&self, pane: PaneHandle) -> Result<bool, String>;

    /// `(selections-charwise? pane)`: no *unambiguous* selection in
    /// `pane`'s own pane is linewise. `true` when every selection is
    /// ambiguous (the complementary default to [`Self::selections_linewise`]'s
    /// `false` in that same case). See [`Self::selections_linewise`] for why
    /// this is a second predicate rather than a verdict field elsewhere.
    fn selections_charwise(&self, pane: PaneHandle) -> Result<bool, String>;
}
