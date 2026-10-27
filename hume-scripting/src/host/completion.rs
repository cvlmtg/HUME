//! Completion: a source's answer in, the ranked view out, accept/dismiss.
//! Registering a source is not a host method: it crosses as
//! `Effect::RegisterCompletionSource` (see `crate::Effect`), so a failed
//! plugin activation's registration is never applied.

use crate::Callback;
use hume_core::{AnswerEnd, FirstTab, MatchKind};

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
    pub proc: Callback,
    pub target: CompletionSourceTarget,
    pub match_kind: MatchKind,
    pub priority: i64,
    /// `#:resolve`: the source's own claim that its items are wire
    /// `CompletionItem`s from the buffer's attached LSP server, so
    /// `completionItem/resolve` may be sent for a selected or accepted one.
    /// `Buffer`-target only; the builtin layer rejects `#t` on a `'minibuf`
    /// source before this is ever constructed (see `builtins/completion.rs`'s
    /// `register_completion_source`).
    pub resolve: bool,
    /// `#:token-chars`: characters that belong to this source's token on
    /// top of the buffer's word characters. `Buffer`-target only, like
    /// `resolve`; empty for a `'minibuf` source.
    pub token_chars: String,
    /// `#:cancel`: called as `(cancel id)` when request `id` ends before
    /// its final answer landed, so the source can stop work nothing will
    /// read. Either target.
    pub cancel: Option<Callback>,
}

/// What a `'minibuf` source's answer says about the word it completes.
/// The default says nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EmittedWord {
    /// `#:span`: the char range of the line the answer completes, in place
    /// of the argument the cursor is in. Only a source registered with
    /// `#:match 'delegated` may name one, and it must hold the cursor
    /// within the line.
    pub span: Option<std::ops::Range<usize>>,
    /// `#:prefix`: what the first Tab puts over the span while no row is
    /// picked. Only a source registered with `#:match 'delegated` may say
    /// anything but [`FirstTab::Common`].
    pub prefix: FirstTab,
}

/// Completion session orchestration, accessed through
/// [`EditorHost::completions`](super::EditorHost::completions).
pub trait CompletionHost {
    /// `(completion-emit! id items #:incomplete f #:more m)`: a source's answer to
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
    /// Returns whether the answer applied: `false` when no source in the
    /// open session still owes an answer to `id` (superseded by a later
    /// keystroke, already answered with an `end` of [`AnswerEnd::Final`],
    /// or the session was replaced or dismissed): expected-normal for a
    /// late async source, never an error. `Err` for a
    /// part that is neither an item nor a response, a caller error, not
    /// dropped.
    ///
    /// `word` is for a `'minibuf` source only, `Err` from a `'buffer` one.
    fn completion_emit(
        &mut self,
        id: u64,
        parts: Vec<crate::json::JsonHandle>,
        incomplete: bool,
        end: AnswerEnd,
        word: EmittedWord,
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
