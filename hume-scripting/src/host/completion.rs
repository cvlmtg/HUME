//! Completion: a source's answer in, the ranked view out, accept/dismiss.
//! Registering a source is not a host method: it crosses as
//! `Effect::RegisterCompletionSource` (see `crate::Effect`), so a failed
//! plugin activation's registration is never applied.

use steel::rvals::SteelVal;

/// How a source's items are matched against its token's typed text,
/// decoded from `register-completion-source!`'s `#:match` symbol
/// (`'fuzzy`/`'string`/`'delegated`) at the builtin layer, converted into
/// the editor's own richer internal enum by the host implementation, same
/// as this crate's `PickerFeedMode`/`TruncateEnd` mirror editor-side
/// concepts at this same boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}

/// `register-completion-source!`'s `#:target`: `'buffer` serves Insert
/// mode (token: the identifier run before the cursor), `'minibuf` the `:`
/// line (token: the whitespace-delimited argument the cursor is in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionSourceTarget {
    Buffer,
    Minibuf,
}

/// A `(register-completion-source! …)` call, queued as an `Effect` for the
/// editor to apply once the eval that made it succeeds.
#[derive(Debug)]
pub struct PendingCompletionSource {
    pub name: String,
    pub proc: SteelVal,
    pub target: CompletionSourceTarget,
    pub match_kind: MatchKind,
    pub priority: i64,
    /// `#:resolve`: the source's own claim that its items are wire
    /// `CompletionItem`s from the buffer's attached LSP server, so
    /// `completionItem/resolve` may be sent for them on accept. `Buffer`-
    /// target only; the builtin layer rejects `#t` on a `'minibuf` source
    /// before this is ever constructed (see `builtins/completion.rs`'s
    /// `register_completion_source`).
    pub resolve: bool,
    /// `#:token-chars`: characters that belong to this source's token on
    /// top of the buffer's word characters. `Buffer`-target only, like
    /// `resolve`; empty for a `'minibuf` source.
    pub token_chars: String,
}

/// Completion session orchestration, accessed through
/// [`EditorHost::completions`](super::EditorHost::completions).
pub trait CompletionHost {
    /// `(completion-emit! id items #:incomplete f)`: a source's answer to
    /// invocation `id`. `parts` is `items`, each element funneled through
    /// `json_arg` (`builtins/completion.rs`), so a response handle crosses
    /// as-is and a hashmap or string becomes a fresh handle. Each part is
    /// one item (a label string, or an object with a `label`) or a whole
    /// `textDocument/completion` response (a `CompletionItem[]` array or a
    /// `CompletionList`), whose items join the others: several servers'
    /// answers to one request become one answer. An empty `parts` means
    /// "nothing from this source". The answer is incomplete when
    /// `incomplete` (the caller's `#:incomplete`) is set or any
    /// `CompletionList` part says so.
    ///
    /// Returns whether the answer applied: `false` when `id` is no longer
    /// the latest call of any source in the open session (superseded by a
    /// later keystroke, or the session was replaced or dismissed):
    /// expected-normal for a late async source, never an error. `Err` for a
    /// part that is neither an item nor a response, a caller error, not
    /// dropped.
    fn completion_emit(
        &mut self,
        id: u64,
        parts: Vec<crate::json::JsonHandle>,
        incomplete: bool,
    ) -> Result<bool, String>;

    /// `(completion-top n)`: up to `n` ranked items as hashmaps, `[]` with
    /// no open session.
    fn completion_top(&self, n: usize) -> Vec<serde_json::Value>;

    /// `(completion-accept! idx)`: applies `idx`'s item (an index into the
    /// ranked/filtered list, not the raw response order) and ends the
    /// session, success or failure.
    fn completion_accept(&mut self, idx: usize) -> Result<(), String>;

    /// `(completion-dismiss!)`: clears any open session, wherever it sits
    /// on the stack; no-op if none is open. A session can be buried (a
    /// picker opened mid-session, unrelated to Insert) without that being
    /// an error, the same "closes regardless of what's on top" contract every
    /// other `close-*!` builtin has.
    fn completion_dismiss(&mut self) -> Result<(), String>;
}
