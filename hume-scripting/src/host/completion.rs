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

/// Where a `Buffer`-target source's token starts — `#:token` on
/// `register-completion-source!` with `#:target 'buffer`. Resolved by the
/// editor against the invocation's own snapshot before the source runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferToken {
    /// `'word` — the identifier run before the cursor.
    Word,
    /// `'cursor` — nothing seeded, nothing replaced before the cursor.
    Cursor,
    /// `'custom` — the answer's own `#:span` names it.
    Custom,
}

/// [`BufferToken`]'s `#:target 'minibuf` counterpart, in byte offsets of
/// the `:` line's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinibufToken {
    /// `'arg` — the whitespace-delimited argument the cursor is in.
    Arg,
    /// `'custom` — the answer's own `#:span` names it.
    Custom,
}

/// `#:target` plus the target's own `#:token` — paired as a type at the
/// builtin so a `'buffer` source with an `'arg` token is a Steel argument
/// error, never a state the editor has to reject later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionSourceTarget {
    Buffer(BufferToken),
    Minibuf(MinibufToken),
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
}

/// Completion session orchestration — accessed through
/// [`EditorHost::completions`](super::EditorHost::completions).
pub trait CompletionHost {
    /// `(completion-emit! id items #:incomplete f #:span (start . end))` —
    /// a source's answer to invocation `id`: `items` is a list of decoded
    /// `CompletionItem` hashmaps (JSON already converted by the caller), an
    /// empty list meaning "nothing from this source". `span` is required
    /// for, and only honoured by, a source registered with a `'custom`
    /// token, in the *invocation's own* coordinates (char offsets of the
    /// buffer the source was called against; bytes of the `input` a
    /// minibuffer source was handed). Returns whether the answer applied —
    /// `false` when `id` is no longer the latest call of any source in the
    /// open session (superseded by a later keystroke, or the session was
    /// replaced or dismissed): expected-normal for a late async source,
    /// never an error. `Err` names an unusable `span`.
    fn completion_emit(
        &mut self,
        id: u64,
        items: Vec<serde_json::Value>,
        incomplete: bool,
        span: Option<(usize, usize)>,
    ) -> Result<bool, String>;

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
