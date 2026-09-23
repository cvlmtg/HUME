//! `EditorHostImpl`'s completion capability: a source's answer in
//! (`completion-emit!`), the ranked view out, accept/dismiss.

use crate::editor::Severity;
use crate::editor::completion::{
    BufferSourceEntry, MinibufBody, MinibufSourceEntry, RegisterOutcome,
};

use super::EditorHostImpl;
use hume_scripting::host::{self, CompletionHost};

/// `hume-scripting`'s `MatchKind` is the Steel/host-trait boundary's own
/// mirror (see its doc) — converted here into the editor's richer internal
/// enum, one-to-one, the same crossing every other Steel-facing option
/// (`PickerFeedMode`, `TruncateEnd`) makes at this same seam.
fn match_kind_from_host(m: host::MatchKind) -> crate::editor::completion::MatchKind {
    use crate::editor::completion::MatchKind as M;
    match m {
        host::MatchKind::Fuzzy => M::Fuzzy,
        host::MatchKind::String { case_sensitive } => M::String { case_sensitive },
        host::MatchKind::Delegated => M::Delegated,
    }
}

impl crate::editor::Editor {
    /// Applies a queued `Effect::RegisterCompletionSource` — a Steel
    /// source entering `ConfigState.completion_sources`. Queued rather
    /// than applied through the host trait inline so a failed plugin
    /// activation's registration is never applied (see `Effect::BindKey`'s
    /// doc for the same reasoning). `#:target` picks the namespace
    /// (`SourceRegistry::register_buffer`/`register_minibuf`) — a
    /// re-registration under a name already taken *in that namespace*
    /// replaces it (a plugin swapping in its own `path` source is a
    /// feature) and says so in the log; the same name in the *other*
    /// namespace is simply a second, unrelated source, since a `Buffer`
    /// trigger and a `TypedCommand.completer` each only ever look in their
    /// own namespace.
    pub(in crate::editor) fn register_completion_source(
        &mut self,
        reg: hume_scripting::PendingCompletionSource,
    ) {
        let name = reg.name;
        let match_kind = match_kind_from_host(reg.match_kind);
        let outcome = match reg.target {
            host::CompletionSourceTarget::Buffer => self
                .state
                .config
                .completion_sources
                .register_buffer(BufferSourceEntry {
                    name: name.clone().into(),
                    match_kind,
                    priority: reg.priority,
                    proc: reg.proc,
                    resolve: reg.resolve,
                    trigger_chars: rustc_hash::FxHashMap::default(),
                }),
            host::CompletionSourceTarget::Minibuf => self
                .state
                .config
                .completion_sources
                .register_minibuf(MinibufSourceEntry {
                    name: name.clone().into(),
                    match_kind,
                    priority: reg.priority,
                    body: MinibufBody::Steel(reg.proc),
                }),
        };
        if outcome == RegisterOutcome::Replaced {
            self.report(
                Severity::Trace,
                format!("register-completion-source!: replaced source {name:?}"),
            );
        }
    }
}

impl<'a> CompletionHost for EditorHostImpl<'a> {
    fn completion_emit(
        &mut self,
        id: u64,
        items: Vec<serde_json::Value>,
        incomplete: bool,
    ) -> bool {
        // A malformed item (missing the spec-required `label`) is skipped,
        // not fatal to the whole batch — one bad item from a misbehaving
        // server must not silently drop every good one.
        let mut parsed = Vec::with_capacity(items.len());
        for v in items {
            match crate::editor::completion::CompletionItem::from_json(v) {
                Some(item) => parsed.push(item),
                None => self.state.report(
                    Severity::Trace,
                    "completion-emit!: skipped malformed item: missing or non-string label"
                        .to_string(),
                ),
            }
        }
        self.state.contribute(self.view, id, parsed, incomplete)
    }

    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        self.state
            .input
            .completion()
            .map(|s| s.top(n, &self.state.config.completion_sources))
            .unwrap_or_default()
    }

    fn completion_accept(&mut self, idx: usize) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-accept!: no LSP state available".to_string());
        };
        // Checked *before* `take_completion_session` — that call is
        // destructive (truncates the layer off the stack, per its own doc),
        // so erroring here first leaves a `Minibuf` session (and its own
        // `minibuf_completion` view slot, which `take_layer` never clears —
        // see `take_completion_session`'s doc) fully intact instead of torn
        // down on a call that was never going to succeed anyway.
        if self
            .state
            .input
            .completion()
            .is_some_and(|s| s.buffer().is_none())
        {
            return Err("completion-accept!: not a buffer-target session".to_string());
        }
        let Some(session) = self.state.take_completion_session(self.view) else {
            return Err("completion-accept!: no active completion session".to_string());
        };
        session.accept(self.state, self.view, lsp, idx)
    }

    fn completion_dismiss(&mut self) -> Result<(), String> {
        self.state.dismiss_completion(self.view);
        Ok(())
    }
}
