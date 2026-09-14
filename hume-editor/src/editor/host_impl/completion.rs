//! `EditorHostImpl`'s completion session orchestration.

use hume_engine::pipeline::BufferId;

use crate::editor::Severity;

use super::EditorHostImpl;
use hume_scripting::host::CompletionHost;

impl<'a> EditorHostImpl<'a> {
    /// Delegates to the shared `clear_completion_menu(state, lsp)` free fn
    /// (`lsp/completion.rs`) — this struct holds disjoint `state`/`lsp`
    /// borrows, not a full `Editor`, so it can't call `Editor`'s method of
    /// the same name, but both now share one body.
    fn clear_completion_menu(&mut self) {
        crate::editor::lsp::completion::clear_completion_menu(self.state, self.lsp.as_deref_mut());
    }
}

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
            // menu, not leave the old one live.
            self.clear_completion_menu();
            self.state
                .report(Severity::Info, "no completions".to_string());
            return Ok(());
        }
        let Some(session) = crate::editor::lsp::completion::CompletionSession::begin(
            self.state, bid, parsed, incomplete,
        ) else {
            // Benign race: the async completion response landed after the
            // user switched away from `bid`'s pane. Not an error — raising
            // here would abort the whole `run_call_batch` this `Call` was
            // batched into and drop every other queued LSP callback/timer
            // batched alongside it.
            self.state.report(
                Severity::Trace,
                "completion-begin!: buffer not shown in focused pane — ignored".to_string(),
            );
            return Ok(());
        };
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-begin!: no LSP state available".to_string());
        };
        lsp.completion = Some(session);
        Ok(())
    }

    fn completion_update_filter(&mut self, text: String) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-update-filter!: no LSP state available".to_string());
        };
        let Some(session) = lsp.completion.as_mut() else {
            return Err("completion-update-filter!: no active completion session".to_string());
        };
        session.update_filter(self.state, text);
        Ok(())
    }

    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        self.lsp
            .as_deref()
            .and_then(|lsp| lsp.completion.as_ref())
            .map(|s| s.top(n))
            .unwrap_or_default()
    }

    fn completion_accept(&mut self, idx: usize) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-accept!: no LSP state available".to_string());
        };
        let Some(session) = lsp.completion.take() else {
            return Err("completion-accept!: no active completion session".to_string());
        };
        // Ends the session either way — success or failure — so a rejected
        // accept never leaves a stale session lingering; the ui/view clear
        // matches `clear_completion_menu`'s scope even though `completion`
        // itself is already `None` here (via `take` above).
        crate::editor::lsp::completion::clear_completion_state(lsp);
        self.state.views.completion_menu.set(None);
        session.accept(self.state, lsp, idx)
    }

    fn completion_dismiss(&mut self) {
        self.clear_completion_menu();
    }
}
