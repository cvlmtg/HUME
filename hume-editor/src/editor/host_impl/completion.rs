//! `EditorHostImpl`'s completion session orchestration.

use hume_engine::pipeline::BufferId;

use crate::editor::Severity;
use crate::editor::input_stack::{CompletionLayer, InsertLayer};

use super::EditorHostImpl;
use hume_scripting::host::CompletionHost;

impl<'a> CompletionHost for EditorHostImpl<'a> {
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        incomplete: bool,
    ) -> Result<(), String> {
        if self.state.buffers.try_get(bid).is_none() {
            return Err("completion-begin!: no such buffer".to_string());
        }
        // A malformed item (e.g. missing the spec-required `label`) is
        // skipped, not fatal to the whole batch — one bad item from a
        // misbehaving server must not silently drop every good one.
        let mut parsed = Vec::with_capacity(items.len());
        for v in &items {
            match crate::editor::lsp::completion::StoredCompletionItem::from_json(v) {
                Ok(item) => parsed.push(item),
                Err(e) => self.state.report(
                    Severity::Trace,
                    format!("completion-begin!: skipped malformed item: {e}"),
                ),
            }
        }
        if parsed.is_empty() {
            // Replaces any open session too — an isIncomplete re-request
            // that comes back empty (or entirely malformed) must close the
            // menu, not leave the old one live. Unconditional: this path
            // never consults the LSP-availability or mode/top gates below,
            // same as before the session moved onto the stack.
            self.state.dismiss_completion(self.view);
            self.state
                .report(Severity::Info, "no completions".to_string());
            return Ok(());
        }
        // Async staleness, same principle as `show-menu!`/`show-drawer-list!`
        // (`input_stack.rs`'s `is_settled_for` doc): the request that led
        // here can land after the user left Insert, or after some *other*
        // modal overlay (a picker opened mid-session — nothing about
        // opening a picker requires Insert) landed on top of it. Either way
        // this is timing, not a plugin bug, so every failure of this gate
        // drops silently rather than erroring — an error here would abort
        // the whole `run_call_batch` this `Call` was batched into. A prior
        // `Completion` instance — buried or not — is the one exception
        // `is_settled_for` tolerates: it's the *normal* refresh path, not an
        // edge — `on-completion-refilter` re-calls this while a session is
        // already open, and so does a trigger char typed with the menu up.
        let mode_ok = self
            .state
            .input
            .is::<InsertLayer>(self.state.input.mode_layer());
        let stack_ok = self.state.input.is_settled_for::<CompletionLayer>();
        if !mode_ok || !stack_ok {
            self.state.report(
                Severity::Trace,
                "completion-begin!: the stack moved before the session could open — ignored"
                    .to_string(),
            );
            return Ok(());
        }
        let Some(session) = crate::editor::lsp::completion::CompletionSession::begin(
            self.state, bid, parsed, incomplete,
        ) else {
            // Benign race: the async completion response landed after the
            // user switched away from `bid`'s pane.
            self.state.report(
                Severity::Trace,
                "completion-begin!: buffer not shown in focused pane — ignored".to_string(),
            );
            return Ok(());
        };
        // The session itself no longer lives on `LspState`, but this
        // builtin has no `require_cmd_ctx!` gate of its own (unlike most
        // command-mode builtins) — `self.lsp` being `None` is the only
        // signal that this call reached an init/lazy-activation eval, which
        // must not push an input layer at all. Checked last, same as
        // before the session moved onto the stack: a benign race (the
        // pane-mismatch `None` case just above) must still win over this,
        // not get masked by it.
        if self.lsp.is_none() {
            return Err("completion-begin!: no LSP state available".to_string());
        }
        // Retires a prior `Completion` instance on the refresh path,
        // wherever it sits — buried under a non-modal `Popup` counts too,
        // now that `is_settled_for` tolerates that above.
        self.state.retire::<CompletionLayer>(self.view);
        self.state
            .push_layer(self.view, CompletionLayer { session, ui: None });
        Ok(())
    }

    fn completion_update_filter(&mut self, text: String) -> Result<(), String> {
        if self.lsp.is_none() {
            return Err("completion-update-filter!: no LSP state available".to_string());
        }
        let Some(bid) = self.state.input.completion().map(|s| s.bid()) else {
            return Err("completion-update-filter!: no active completion session".to_string());
        };
        // Read from `state.buffers` before re-borrowing the session
        // mutably out of `state.input` — the two now live inside the same
        // top-level struct, so a `&EditorState` passed alongside a `&mut
        // CompletionSession` borrowed from it would alias.
        let text_gen = self.state.buffers.get(bid).text_gen;
        let session = self.state.input.completion_mut().expect("checked above");
        session.update_filter(text_gen, text);
        Ok(())
    }

    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        self.state
            .input
            .completion()
            .map(|s| s.top(n))
            .unwrap_or_default()
    }

    fn completion_accept(&mut self, idx: usize) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-accept!: no LSP state available".to_string());
        };
        let Some(session) = self.state.take_completion_session(self.view) else {
            return Err("completion-accept!: no active completion session".to_string());
        };
        session.accept(self.state, lsp, idx)
    }

    fn completion_dismiss(&mut self) -> Result<(), String> {
        self.state.dismiss_completion(self.view);
        Ok(())
    }
}
