//! `EditorHostImpl`'s completion capability: a source's answer in
//! (`completion-emit!`), the ranked view out, accept/dismiss.

use crate::editor::Severity;
use crate::editor::completion::{
    BufferSourceEntry, MinibufBody, MinibufSourceEntry, RegisterOutcome,
};

use super::EditorHostImpl;
use hume_scripting::host::{self, CompletionHost};

/// `hume-scripting`'s `MatchKind` is the Steel/host-trait boundary's own
/// mirror (see its doc), converted here into the editor's richer internal
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
    /// Applies a queued `Effect::RegisterCompletionSource`: a Steel
    /// source entering `ConfigState.completion_sources`. Queued rather
    /// than applied through the host trait inline so a failed plugin
    /// activation's registration is never applied (see `Effect::BindKey`'s
    /// doc for the same reasoning). `#:target` picks the namespace
    /// (`SourceRegistry::register_buffer`/`register_minibuf`); a
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
                    token_chars: reg.token_chars.into(),
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
        parts: Vec<hume_scripting::json::JsonHandle>,
        incomplete: bool,
    ) -> Result<bool, String> {
        // `hume-scripting` hands each part over as an opaque `JsonHandle`;
        // telling an item from a response happens here, the one place that
        // depends on both `hume_lsp` and this store's `CompletionItem`.
        let mut incomplete = incomplete;
        let mut parsed = Vec::new();
        for part in parts {
            let value = part.value();
            let is_item = value.is_string() || value.get("label").is_some();
            if is_item {
                self.parse_item(value, part.clone(), None, &mut parsed);
                continue;
            }
            // `items_key`: `Some("items")` for a `CompletionList` (its items
            // live under that key), `None` for a bare `CompletionItem[]`,
            // so each item's `indexed_child` walks the right path.
            let (items, own_incomplete, items_key) =
                match hume_lsp::completion_item::completion_response_items(value) {
                    Some((items, Some(own))) => (items, own, Some("items")),
                    Some((items, None)) => (items, false, None),
                    None => {
                        return Err(
                            "completion-emit!: an item must be a label string or an object with \
                             a label, or a textDocument/completion response (a CompletionItem[] \
                             array or a CompletionList object)"
                                .to_string(),
                        );
                    }
                };
            incomplete |= own_incomplete;
            let default_range = hume_lsp::completion_item::item_defaults_edit_range(value);
            for (i, v) in items.iter().enumerate() {
                let raw_item = part
                    .indexed_child(items_key, i)
                    .expect("index i is within items, already resolved from the part's value");
                self.parse_item(v, raw_item, default_range.as_ref(), &mut parsed);
            }
        }
        Ok(self.state.contribute(self.view, id, parsed, incomplete))
    }

    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        let sources = &self.state.config.completion_sources;
        self.state
            .input
            .buffer_completion()
            .map(|s| s.top(n, sources))
            .or_else(|| {
                self.state
                    .input
                    .minibuf_completion()
                    .map(|s| s.top(n, sources))
            })
            .unwrap_or_default()
    }

    fn completion_accept(&mut self, idx: usize) -> Result<(), String> {
        self.state.refuse_during_dot("completion-accept!")?;
        // Checked *before* `take_buffer_completion`: that call is
        // destructive (truncates the layer off the stack, per its own doc),
        // so erroring here first leaves a `Minibuf` session (and its own
        // `minibuf_completion` view slot, which `take_layer` never clears;
        // see `take_buffer_completion`'s doc) fully intact instead of torn
        // down on a call that was never going to succeed anyway.
        if self.state.input.minibuf_completion().is_some() {
            return Err("completion-accept!: not a buffer-target session".to_string());
        }
        let Some(session) = self.state.take_buffer_completion(self.view) else {
            return Err("completion-accept!: no active completion session".to_string());
        };
        session.accept(self.state, self.view, idx)
    }

    fn completion_dismiss(&mut self) -> Result<(), String> {
        self.state.dismiss_completion(self.view);
        Ok(())
    }
}

impl EditorHostImpl<'_> {
    /// Decodes one item into `parsed`. A malformed item (missing the
    /// spec-required `label`) is skipped, not fatal to the whole answer: one
    /// bad item from a misbehaving server must not drop every good one.
    fn parse_item(
        &mut self,
        value: &serde_json::Value,
        raw_item: hume_scripting::json::JsonHandle,
        default_range: Option<&lsp_types::Range>,
        parsed: &mut Vec<crate::editor::completion::CompletionItem>,
    ) {
        match crate::editor::completion::CompletionItem::from_json(value, raw_item, default_range) {
            Some(item) => parsed.push(item),
            None => self.state.report(
                Severity::Trace,
                "completion-emit!: skipped malformed item: missing or non-string label".to_string(),
            ),
        }
    }
}
