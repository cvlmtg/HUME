//! Trigger characters plugins register, of two kinds ([`TriggerKind`]) and
//! two scopes ([`TriggerScope`]): the buffers of a language
//! (`ConfigState::trigger_chars`), or one buffer's attachment to one
//! server, which goes when the buffer detaches (`lsp/document.rs`). Every
//! write goes through [`EditorState::set_triggers`] and every read through
//! [`EditorState::trigger_sources_for`].

use hume_engine::pipeline::BufferId;
use hume_scripting::{TriggerKind, TriggerScope};

use super::EditorState;
use super::completion::BufferSourceId;

/// Sets `key`'s trigger characters in `map`, replacing its previous set;
/// an empty `chars` removes the entry rather than leaving an empty one.
pub(in crate::editor) fn set_trigger_chars<K: std::hash::Hash + Eq>(
    map: &mut rustc_hash::FxHashMap<K, Vec<char>>,
    key: K,
    chars: Vec<char>,
) {
    if chars.is_empty() {
        map.remove(&key);
    } else {
        map.insert(key, chars);
    }
}

impl EditorState {
    /// Sets `source`'s trigger characters of `kind` in `scope`, replacing
    /// that triple's previous set. A `Completion` set must name a
    /// registered `Buffer` source: a plugin's own typo or stale rename, not
    /// something to ignore the way a `Hook` set has to (a hook's listener
    /// need not be a completion source). An attachment scope for a buffer
    /// not attached to the server sets nothing.
    pub(in crate::editor) fn set_triggers(
        &mut self,
        kind: TriggerKind,
        source: String,
        scope: TriggerScope,
        chars: Vec<char>,
    ) -> Result<(), String> {
        if kind == TriggerKind::Completion {
            self.config.completion_sources.trigger_source(&source)?;
        }
        match scope {
            TriggerScope::Language(language) => set_trigger_chars(
                &mut self.config.trigger_chars,
                (kind, source, language),
                chars,
            ),
            TriggerScope::Attachment { buffer, server } => self
                .buffer_positions
                .lsp
                .set_triggers(buffer, server, kind, source, chars),
        }
        Ok(())
    }

    /// Every source with trigger characters of `kind` that include `ch` in
    /// `bid`: the ones set for `bid`'s language and the ones set for a
    /// server attached to it, each source once.
    pub(in crate::editor) fn trigger_sources_for(
        &self,
        kind: TriggerKind,
        ch: char,
        bid: BufferId,
    ) -> Vec<String> {
        let language = self
            .buffers
            .try_get(bid)
            .and_then(|b| b.language)
            .map(|id| self.config.languages.name_of(id));
        let by_language = self
            .config
            .trigger_chars
            .iter()
            .filter(|((k, _, lang), chars)| {
                *k == kind && Some(lang.as_str()) == language && chars.contains(&ch)
            })
            .map(|((_, source, _), _)| source.as_str());
        let by_server = self
            .buffer_positions
            .lsp
            .triggers(bid, kind)
            .filter(|(_, chars)| chars.contains(&ch))
            .map(|(source, _)| source);
        let mut sources: Vec<String> = Vec::new();
        for source in by_language.chain(by_server) {
            if !sources.iter().any(|s| s == source) {
                sources.push(source.to_owned());
            }
        }
        sources
    }

    /// The `Buffer` completion sources a typed `ch` invokes in `bid`, in
    /// registration order. A trigger set naming no registered source
    /// matches nothing.
    pub(in crate::editor) fn completion_sources_for_trigger(
        &self,
        ch: char,
        bid: BufferId,
    ) -> Vec<BufferSourceId> {
        let names = self.trigger_sources_for(TriggerKind::Completion, ch, bid);
        self.config.completion_sources.buffer_sources_named(&names)
    }
}
