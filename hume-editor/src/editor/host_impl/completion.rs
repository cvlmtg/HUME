//! `EditorHostImpl`'s completion capability: a source's answer in
//! (`completion-emit!`), the ranked view out, accept/dismiss.

use crate::editor::Severity;
use crate::editor::completion::{BufferToken, MinibufToken, SourceBody, SourceEntry, SourceTarget};

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

fn target_from_host(t: host::CompletionSourceTarget) -> SourceTarget {
    match t {
        host::CompletionSourceTarget::Buffer(token) => SourceTarget::Buffer(match token {
            host::BufferToken::Word => BufferToken::Word,
            host::BufferToken::Cursor => BufferToken::Cursor,
            host::BufferToken::Custom => BufferToken::Custom,
        }),
        host::CompletionSourceTarget::Minibuf(token) => SourceTarget::Minibuf(match token {
            host::MinibufToken::Arg => MinibufToken::Arg,
            host::MinibufToken::Custom => MinibufToken::Custom,
        }),
    }
}

impl crate::editor::Editor {
    /// Applies a queued `Effect::RegisterCompletionSource` — a Steel
    /// source entering `ConfigState.completion_sources`. Queued rather
    /// than applied through the host trait inline so a failed plugin
    /// activation's registration is never applied (see `Effect::BindKey`'s
    /// doc for the same reasoning); a re-registration under a taken name,
    /// native or Steel, replaces it — a plugin swapping in its own `path`
    /// source is a feature — and says so in the log.
    pub(in crate::editor) fn register_completion_source(
        &mut self,
        reg: hume_scripting::PendingCompletionSource,
    ) {
        let name = reg.name;
        let replaced = self.state.config.completion_sources.register(SourceEntry {
            name: name.clone().into(),
            match_kind: match_kind_from_host(reg.match_kind),
            priority: reg.priority,
            body: SourceBody::Steel {
                proc: reg.proc,
                target: target_from_host(reg.target),
            },
        });
        if replaced {
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
        span: Option<(usize, usize)>,
    ) -> Result<bool, String> {
        // A malformed item (e.g. missing the spec-required `label`) is
        // skipped, not fatal to the whole batch — one bad item from a
        // misbehaving server must not silently drop every good one.
        let mut parsed = Vec::with_capacity(items.len());
        for v in &items {
            match crate::editor::completion::CompletionItem::from_json(v) {
                Ok(item) => parsed.push(item),
                Err(e) => self.state.report(
                    Severity::Trace,
                    format!("completion-emit!: skipped malformed item: {e}"),
                ),
            }
        }
        self.state
            .contribute(self.view, id, parsed, incomplete, span)
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
