//! Completion session orchestration.

use hume_engine::pipeline::BufferId;

/// Completion session orchestration — accessed through
/// [`EditorHost::completions`](super::EditorHost::completions).
pub trait CompletionHost {
    /// `(completion-begin! bid items #:incomplete f)` — `items` is a list of
    /// decoded `CompletionItem` hashmaps (JSON already converted by the
    /// caller). Starting a session replaces any session already open.
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        incomplete: bool,
    ) -> Result<(), String>;

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
