//! `EditorHostImpl`'s completion session orchestration.

use hume_engine::pipeline::BufferId;

use crate::editor::Severity;
use crate::editor::input_stack::{self, CompletionLayer, InsertLayer};

use super::EditorHostImpl;
use hume_scripting::host::CompletionHost;

/// `hume-scripting`'s `MatchKind` is the Steel/host-trait boundary's own
/// mirror (see its doc) — converted here into the editor's richer internal
/// enum, one-to-one, the same crossing every other Steel-facing option
/// (`PickerFeedMode`, `TruncateEnd`) makes at this same seam.
fn match_kind_from_host(
    m: hume_scripting::host::MatchKind,
) -> crate::editor::completion::MatchKind {
    use crate::editor::completion::MatchKind as M;
    match m {
        hume_scripting::host::MatchKind::Fuzzy => M::Fuzzy,
        hume_scripting::host::MatchKind::String { case_sensitive } => M::String { case_sensitive },
        hume_scripting::host::MatchKind::Delegated => M::Delegated,
    }
}

/// Parses `items` via `CompletionItem::from_json`, skipping (not failing on)
/// a malformed one — shared by `completion_begin` and `completion_add_items`
/// so the two never drift on this tolerance. `builtin_name` is only for the
/// skip's own Trace line.
fn parse_items(
    state: &mut crate::editor::EditorState,
    items: &[serde_json::Value],
    builtin_name: &str,
) -> Vec<crate::editor::completion::CompletionItem> {
    let mut parsed = Vec::with_capacity(items.len());
    for v in items {
        match crate::editor::completion::CompletionItem::from_json(v) {
            Ok(item) => parsed.push(item),
            Err(e) => state.report(
                Severity::Trace,
                format!("{builtin_name}: skipped malformed item: {e}"),
            ),
        }
    }
    parsed
}

impl<'a> CompletionHost for EditorHostImpl<'a> {
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        source: String,
        priority: i64,
        match_kind: hume_scripting::host::MatchKind,
        incomplete: bool,
    ) -> Result<u64, String> {
        let match_kind = match_kind_from_host(match_kind);
        if self.state.buffers.try_get(bid).is_none() {
            return Err("completion-begin!: no such buffer".to_string());
        }
        // Async staleness — see `EditorState::async_opener_stale`'s own doc.
        // Checked before anything else touches the stack or parses a single
        // item: a stale response (the mode layer changed, or a modal
        // overlay landed on top, since the request went out) must leave
        // whatever's already there untouched — including an *empty* stale
        // response, which must not dismiss a session this response knows
        // nothing about. A prior `Completion` instance — buried or not — is
        // the one exception this gate tolerates: it's the *normal* refresh
        // path, not an edge — `on-completion-refilter` re-calls this while a
        // session is already open, and so does a trigger char typed with
        // the menu up.
        if self
            .state
            .async_opener_stale::<InsertLayer, CompletionLayer>("completion-begin!")
        {
            return Ok(crate::editor::widget_token::DEAD);
        }
        // A malformed item (e.g. missing the spec-required `label`) is
        // skipped, not fatal to the whole batch — one bad item from a
        // misbehaving server must not silently drop every good one.
        let parsed = parse_items(self.state, &items, "completion-begin!");
        if parsed.is_empty() {
            // Replaces any open session too — an isIncomplete re-request
            // that comes back empty (or entirely malformed) must close the
            // menu, not leave the old one live.
            self.state.dismiss_completion(self.view);
            self.state
                .report(Severity::Info, "no completions".to_string());
            // No session opened — there is no token to hand back.
            // `widget_token::DEAD` is never a live token, so a caller that
            // (wrongly) tries to `completion-add-items!` against it gets the
            // same silent no-op a real stale token would.
            return Ok(crate::editor::widget_token::DEAD);
        }
        let Some(session) = crate::editor::completion::CompletionSession::begin_buffer(
            self.state,
            bid,
            source.into(),
            priority,
            match_kind,
            parsed,
            incomplete,
        ) else {
            // Benign race: the async completion response landed after the
            // user switched away from `bid`'s pane.
            self.state.report(
                Severity::Trace,
                "completion-begin!: buffer not shown in focused pane — ignored".to_string(),
            );
            return Ok(0);
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
        let token = session.token();
        // Retires a prior `Completion` instance on the refresh path,
        // wherever it sits — buried under a non-modal `Popup` counts too,
        // now that `is_settled_for` tolerates that above.
        self.state.retire::<CompletionLayer>(self.view);
        self.state
            .push_layer(self.view, CompletionLayer { session, ui: None });
        Ok(token)
    }

    fn completion_add_items(
        &mut self,
        token: u64,
        items: Vec<serde_json::Value>,
        source: String,
        priority: i64,
        match_kind: hume_scripting::host::MatchKind,
        incomplete: bool,
    ) -> bool {
        let match_kind = match_kind_from_host(match_kind);
        // Token checked before parsing anything — a stale token is a
        // silent no-op end to end, including no Trace noise from parsing
        // items nothing will ever use. Checked twice: this first lookup is
        // discarded (its only job is the early liveness check), then
        // `add_items` reaches through a second, fresh one — parsing sits
        // between them so a stale token skips it entirely.
        if input_stack::completion::session_for_token(self.state, token).is_none() {
            return false;
        }
        let parsed = parse_items(self.state, &items, "completion-add-items!");
        let Some(session) = input_stack::completion::session_for_token(self.state, token) else {
            return false;
        };
        session.add_items(source.into(), priority, match_kind, incomplete, parsed);
        self.state.reset_completion_selection();
        true
    }

    fn completion_update_filter(&mut self, text: String) -> Result<(), String> {
        if self.lsp.is_none() {
            return Err("completion-update-filter!: no LSP state available".to_string());
        }
        let Some(session) = self.state.input.completion_mut() else {
            return Err("completion-update-filter!: no active completion session".to_string());
        };
        session.update_filter(text);
        self.state.reset_completion_selection();
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
        session.accept(self.state, self.view, lsp, idx)
    }

    fn completion_dismiss(&mut self) -> Result<(), String> {
        self.state.dismiss_completion(self.view);
        Ok(())
    }
}
