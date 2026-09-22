//! Completion: a source's answer in, the ranked view out, accept/dismiss.
//! Registering a source is not a host method — it crosses as
//! `Effect::RegisterCompletionSource` (see `crate::Effect`), so a failed
//! plugin activation's registration is never applied.

use steel::rvals::SteelVal;

/// How a source's items are matched against its token's typed text —
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

/// `register-completion-source!`'s `#:target` — `'buffer` serves Insert
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
    /// `#:resolve` — the source's own claim that its items are wire
    /// `CompletionItem`s from the buffer's attached LSP server, so
    /// `completionItem/resolve` may be sent for them on accept. `Buffer`-
    /// target only; the builtin layer rejects `#t` on a `'minibuf` source
    /// before this is ever constructed (see `builtins/completion.rs`'s
    /// `register_completion_source`).
    pub resolve: bool,
}

/// Completion session orchestration — accessed through
/// [`EditorHost::completions`](super::EditorHost::completions).
pub trait CompletionHost {
    /// `(completion-emit! id items #:incomplete f)` — a source's answer to
    /// invocation `id`: `items` is a list of decoded `CompletionItem`
    /// hashmaps (JSON already converted by the caller), an empty list
    /// meaning "nothing from this source". Returns whether the answer
    /// applied — `false` when `id` is no longer the latest call of any
    /// source in the open session (superseded by a later keystroke, or the
    /// session was replaced or dismissed): expected-normal for a late async
    /// source, never an error.
    fn completion_emit(&mut self, id: u64, items: Vec<serde_json::Value>, incomplete: bool)
    -> bool;

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

    /// `(completion-set-trigger-chars! source language chars)` — registers
    /// `chars` as `source`'s own trigger characters for `language`,
    /// replacing that exact `(source, language)` pair's previous set; an
    /// empty `chars` removes it. This is `'buffer` sources' own routing
    /// table, separate from `register-trigger-chars!`'s shared,
    /// listener-agnostic one (`LanguageHost::register_trigger_chars`) —
    /// only a registered `'buffer` source's own trigger chars decide which
    /// sources a keystroke invokes (`Trigger::Char`, editor-side); `Err`
    /// when `source` names no registered `'buffer` source, a plugin's own
    /// typo or stale rename.
    fn completion_set_trigger_chars(
        &mut self,
        source: &str,
        language: &str,
        chars: Vec<char>,
    ) -> Result<(), String>;
}
