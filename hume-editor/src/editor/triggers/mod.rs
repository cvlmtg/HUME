//! Trigger characters plugins register: the typed characters that fire a
//! [`Listener`], an `on-trigger-char` hook name or a `'buffer` completion
//! source (`set-hook-triggers!`, `set-completion-triggers!` and their
//! attachment forms).
//!
//! A [`TriggerTable`] holds one scope's sets. There are two kinds of scope:
//! the buffers of a language (`ConfigState::triggers`, keyed by language
//! name so a set may precede the language's definition), and one buffer's
//! attachment to one server, whose table goes when the buffer detaches or
//! the attachment's filter changes (`lsp/document.rs`).
//!
//! Every write goes through [`EditorState::set_triggers`]. Every read goes
//! through [`EditorState::triggered_by`]: the union of the buffer's
//! language table and its attachments' tables, each listener once.

use std::sync::Arc;

use hume_engine::pipeline::BufferId;
use hume_scripting::{TriggerKind, TriggerScope};

mod table;

pub(in crate::editor) use table::{Listener, TriggerTable};

use super::EditorState;
use super::completion::BufferSourceId;

/// What a typed character fires in one buffer.
#[derive(Debug, Default)]
pub(in crate::editor) struct Triggered {
    /// Hook names, in name order.
    pub(in crate::editor) hooks: Vec<Arc<str>>,
    /// Registered `'buffer` completion sources, in registration order.
    pub(in crate::editor) completions: Vec<BufferSourceId>,
}

impl EditorState {
    /// Sets `source`'s trigger characters of `kind` in `scope`, replacing
    /// that listener's previous set; no characters removes it. A
    /// `Completion` set must name a registered `Buffer` source: a plugin's
    /// own typo or stale rename, not something to ignore the way a `Hook`
    /// set has to (a hook's listener need not be a completion source). An
    /// attachment scope sets nothing unless the server handles its feature
    /// for the buffer.
    pub(in crate::editor) fn set_triggers(
        &mut self,
        kind: TriggerKind,
        source: String,
        scope: TriggerScope,
        chars: Vec<char>,
    ) -> Result<(), String> {
        let listener = match kind {
            TriggerKind::Hook => Listener::Hook(source.into()),
            TriggerKind::Completion => {
                self.config.completion_sources.trigger_source(&source)?;
                Listener::Completion(source.into())
            }
        };
        match scope {
            TriggerScope::Language(language) => {
                let language: Box<str> = language.into();
                let table = self.config.triggers.entry(language.clone()).or_default();
                table.set(listener, chars);
                if table.is_empty() {
                    self.config.triggers.remove(&language);
                }
            }
            TriggerScope::Attachment {
                buffer,
                server,
                feature,
            } => {
                if self.lsp_handles(buffer, server, feature) {
                    self.buffer_positions
                        .lsp
                        .set_triggers(buffer, server, listener, chars);
                }
            }
        }
        Ok(())
    }

    /// What `ch` fires in `bid`: the listeners of its language's table and
    /// of its attachments' tables, each once. A completion listener whose
    /// source is not registered fires nothing.
    pub(in crate::editor) fn triggered_by(&self, ch: char, bid: BufferId) -> Triggered {
        let language_table = self
            .buffers
            .try_get(bid)
            .and_then(|buffer| buffer.language)
            .and_then(|id| self.config.triggers.get(self.config.languages.name_of(id)));
        let mut listeners: Vec<&Listener> = language_table
            .into_iter()
            .chain(self.buffer_positions.lsp.trigger_tables(bid))
            .flat_map(|table| table.fires_on(ch))
            .collect();
        listeners.sort_unstable();
        listeners.dedup();

        let mut triggered = Triggered::default();
        for listener in listeners {
            match listener {
                Listener::Hook(name) => triggered.hooks.push(Arc::clone(name)),
                Listener::Completion(name) => triggered
                    .completions
                    .extend(self.config.completion_sources.buffer_id_of(name)),
            }
        }
        triggered.completions.sort_unstable();
        triggered.completions.dedup();
        triggered
    }
}
