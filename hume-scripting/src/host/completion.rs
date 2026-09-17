//! Completion session orchestration.

use hume_engine::pipeline::BufferId;

/// How a source's items are matched against the typed filter — decoded from
/// `completion-begin!`/`completion-add-items!`'s `#:match` symbol
/// (`'fuzzy`/`'string`/`'delegated`) at the builtin layer, converted into the
/// editor's own richer internal enum by the host implementation. Only
/// `Fuzzy` has a real Steel caller today (`core:lsp`); `String`/`Delegated`
/// exist for a future Steel-registered source to declare, same as this
/// crate's `PickerFeedMode`/`TruncateEnd` mirror editor-side concepts at this
/// same boundary.
pub enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}

/// What further typing does while the popup is open — decoded from
/// `#:interaction` (`'select`/`'cycle`). Only `SelectAccept` has a real
/// Steel caller today.
pub enum Interaction {
    CycleApply,
    SelectAccept,
}

/// Completion session orchestration — accessed through
/// [`EditorHost::completions`](super::EditorHost::completions).
pub trait CompletionHost {
    /// `(completion-begin! bid items #:source s #:incomplete f #:priority n
    /// #:match k #:interaction i)` — `items` is a list of decoded
    /// `CompletionItem` hashmaps (JSON already converted by the caller),
    /// tagged with the contributing `source`'s name. Starting a session
    /// replaces any session already open. Returns the new session's token
    /// (`0` if no session was opened — an empty/all-malformed `items`), for
    /// a later `completion-add-items!` to merge a second source into.
    #[allow(clippy::too_many_arguments)]
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        source: String,
        priority: i64,
        match_kind: MatchKind,
        interaction: Interaction,
        incomplete: bool,
    ) -> Result<u64, String>;

    /// `(completion-add-items! token items #:source s #:priority n #:match k
    /// #:incomplete f)` — merges `items` into the session `token` names,
    /// replacing that source's prior contribution wholesale (an
    /// `isIncomplete` re-request re-emitting the same source is therefore
    /// idempotent, not additive) and re-ranking. A mismatched `token` — the
    /// session was replaced or dismissed since the caller captured it — is
    /// expected-normal, not an error: returns whether the merge applied,
    /// same silent-no-op contract as `picker-push!`/`picker-replace!`.
    /// `interaction` isn't a parameter here — it's decided once, by whoever
    /// calls `completion_begin`, and stays fixed for the session's life.
    #[allow(clippy::too_many_arguments)]
    fn completion_add_items(
        &mut self,
        token: u64,
        items: Vec<serde_json::Value>,
        source: String,
        priority: i64,
        match_kind: MatchKind,
        incomplete: bool,
    ) -> bool;

    /// `(completion-update-filter! text)` — re-ranks the open session
    /// against `text`; Rust-side work only, safe to call every keystroke.
    fn completion_update_filter(&mut self, text: String) -> Result<(), String>;

    /// `(completion-top n)` — up to `n` ranked items as hashmaps, `[]` with
    /// no open session.
    fn completion_top(&self, n: usize) -> Vec<serde_json::Value>;

    /// `(completion-accept! idx)` — applies `idx`'s item (an index into the
    /// ranked/filtered list, not the raw response order) and ends the
    /// session, success or failure.
    fn completion_accept(&mut self, idx: usize) -> Result<(), String>;

    /// `(completion-dismiss!)` — clears any open session, wherever it sits
    /// on the stack; no-op if none is open. A session can be buried (a
    /// picker opened mid-session, unrelated to Insert) without that being
    /// an error — same "closes regardless of what's on top" contract every
    /// other `close-*!` builtin has.
    fn completion_dismiss(&mut self) -> Result<(), String>;
}
