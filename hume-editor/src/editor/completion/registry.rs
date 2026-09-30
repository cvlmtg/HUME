//! Every completion source the editor knows: the six native minibuffer
//! sources compiled in, plus whatever Steel registered via
//! `(register-completion-source! …)`. **Two separate namespaces, one per
//! target**: a `TypedCommand.completer` names an entry in [`SourceRegistry::minibuf`],
//! an Insert-mode trigger invokes entries in [`SourceRegistry::buffer`]. A
//! name can be taken in both namespaces at once: `"path"` as a `Buffer`
//! source and `"path"` as a `Minibuf` source are two unrelated
//! registrations, not a collision, because there is no shared id space for
//! them to collide in. Lives on `ConfigState`, so `:reload-config` rebuilds
//! it from the natives by construction and a Steel entry never outlives the
//! config that registered it.
//!
//! A source's *static* facts live here (its name, how its items are scored,
//! its priority); everything about one particular invocation of it (the
//! document it saw, the span it answered for, the items) is `session.rs`'s
//! [`super::Invocation`]. A session refers back here by id rather than
//! copying any of this, so a name/priority/match kind has exactly one home.
//!
//! Every `Buffer` source is Steel. There is no native one, by design: a
//! `Buffer`-target source scans live editor state (the buffer, an LSP
//! server), which is exactly what the plugin layer exists to reach into.

use std::ops::Range;

use steel::rvals::SteelVal;

use super::{CompletionCtx, CompletionItem, MatchKind};

/// A native minibuffer source that enumerates a stable universe: the
/// session's own matcher narrows it, and the orchestrator supplies the
/// `'arg` span it never has to compute.
pub(in crate::editor) type NativeUniverseFn = fn(&CompletionCtx<'_>) -> Vec<CompletionItem>;

/// A native minibuffer source whose universe *is* the live input (a
/// directory listing, a parse phase): computes its own finished, ordered
/// result and its own byte span, fresh every attempt.
pub(in crate::editor) type NativeDelegatedFn =
    fn(&str, usize, &CompletionCtx<'_>) -> (Range<usize>, Vec<CompletionItem>);

/// How a `Minibuf` source produces its answer: the two native shapes each
/// have a fixed span rule of their own (see [`super::orchestrate::
/// invoke_minibuf_source`]'s doc): `NativeDelegated` computes its own,
/// everything else (including every `Steel` minibuf source) gets the
/// whitespace-delimited `'arg` span computed upfront by the orchestrator.
pub(in crate::editor) enum MinibufBody {
    NativeUniverse(NativeUniverseFn),
    NativeDelegated(NativeDelegatedFn),
    /// Invoked via `EditorState::queue_steel_call` as `(proc id input
    /// cursor)`, answered by `(completion-emit! id …)`.
    Steel(SteelVal),
}

pub(in crate::editor) struct BufferSourceEntry {
    pub(in crate::editor) name: Box<str>,
    pub(in crate::editor) match_kind: MatchKind,
    pub(in crate::editor) priority: i64,
    /// Invoked via `EditorState::queue_steel_call` as `(proc id bid
    /// prefix)`, answered by `(completion-emit! id …)`.
    pub(in crate::editor) proc: SteelVal,
    /// `#:resolve`: this source's own claim that its items are wire
    /// `CompletionItem`s from the buffer's attached LSP server, so
    /// `completionItem/resolve` may be sent for an accepted one on its
    /// behalf (`session/accept.rs`'s `maybe_send_resolve`). `false` by
    /// default: a source that didn't ask for this (`core:buffer-words`, any
    /// other synthetic-item source) never has a resolve request sent for
    /// its items, however the buffer's own LSP server capabilities read.
    pub(in crate::editor) resolve: bool,
    /// `#:token-chars`: characters that belong to this source's token on top
    /// of the buffer's word characters ([`Self::token_chars_over`]).
    pub(in crate::editor) token_chars: Box<str>,
    /// This source's own trigger characters, per language, set by
    /// `(set-completion-triggers! name language chars)`
    /// ([`SourceRegistry::set_buffer_trigger_chars`]), not at registration
    /// time (a server's own trigger characters usually aren't known until
    /// it attaches). Empty until then; an empty `chars` for a language
    /// removes that language's entry rather than leaving a stale empty one.
    pub(in crate::editor) trigger_chars: rustc_hash::FxHashMap<String, Vec<char>>,
}

impl BufferSourceEntry {
    /// The characters classified as `Word` while scanning this source's
    /// token: the buffer's `word_chars` plus this source's own
    /// `token_chars`. Borrows `word_chars` when the source adds none.
    pub(in crate::editor) fn token_chars_over<'a>(
        &'a self,
        word_chars: &'a str,
    ) -> std::borrow::Cow<'a, str> {
        if self.token_chars.is_empty() {
            std::borrow::Cow::Borrowed(word_chars)
        } else {
            std::borrow::Cow::Owned(format!("{word_chars}{}", self.token_chars))
        }
    }
}

pub(in crate::editor) struct MinibufSourceEntry {
    pub(in crate::editor) name: Box<str>,
    pub(in crate::editor) match_kind: MatchKind,
    pub(in crate::editor) priority: i64,
    pub(in crate::editor) body: MinibufBody,
}

/// Index into [`SourceRegistry::buffer`], stable for an entry's whole
/// life, since a re-registration under the same name overwrites the slot in
/// place rather than pushing a new one. Distinct from [`MinibufSourceId`]
/// so a caller can't hand a buffer-namespace id to a minibuf-namespace
/// lookup (or the reverse): the two id spaces don't overlap, and there is
/// no shared "which target" tag left to check at runtime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) struct BufferSourceId(u32);

/// [`BufferSourceId`]'s counterpart into [`SourceRegistry::minibuf`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) struct MinibufSourceId(u32);

pub(in crate::editor) struct SourceRegistry {
    buffer: Vec<BufferSourceEntry>,
    minibuf: Vec<MinibufSourceEntry>,
}

/// What a `register_*` call did. The caller's own report depends on which.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum RegisterOutcome {
    Added,
    Replaced,
}

impl SourceRegistry {
    pub(in crate::editor) fn with_defaults() -> Self {
        use super::{path, set, simple};
        let universe = |name: &str, case_sensitive, body| MinibufSourceEntry {
            name: name.into(),
            match_kind: MatchKind::String { case_sensitive },
            priority: 0,
            body: MinibufBody::NativeUniverse(body),
        };
        let delegated = |name: &str, body| MinibufSourceEntry {
            name: name.into(),
            match_kind: MatchKind::Delegated,
            priority: 0,
            body: MinibufBody::NativeDelegated(body),
        };
        Self {
            buffer: Vec::new(),
            minibuf: vec![
                universe(simple::COMMAND_SOURCE, false, simple::complete_command),
                universe(
                    simple::BUFFER_NAME_SOURCE,
                    true,
                    simple::complete_buffer_name,
                ),
                universe(simple::THEME_SOURCE, true, simple::complete_theme),
                delegated(path::PATH_SOURCE, path::complete_path),
                delegated(path::PATH_DIRS_ONLY_SOURCE, path::complete_path_dirs_only),
                delegated(set::SET_SOURCE, set::complete_set),
            ],
        }
    }

    /// Registers (or, under an already-taken name, replaces in place) a
    /// `Buffer` source. Replacing keeps the entry's [`BufferSourceId`]
    /// stable for any session still referring to it (a plugin swapping in
    /// its own source under a name it already owns is a feature, not a
    /// collision), and carries the old entry's `trigger_chars` forward:
    /// that routing table is set by a separate call
    /// ([`Self::set_buffer_trigger_chars`]), at a different time than
    /// registration, so a `proc`/`match_kind`/`priority`/`resolve` swap has
    /// no business clearing it.
    pub(in crate::editor) fn register_buffer(
        &mut self,
        entry: BufferSourceEntry,
    ) -> RegisterOutcome {
        match self.buffer_id_of(&entry.name) {
            Some(id) => {
                let trigger_chars = std::mem::take(&mut self.buffer[id.0 as usize].trigger_chars);
                self.buffer[id.0 as usize] = BufferSourceEntry {
                    trigger_chars,
                    ..entry
                };
                RegisterOutcome::Replaced
            }
            None => {
                self.buffer.push(entry);
                RegisterOutcome::Added
            }
        }
    }

    /// Sets `name`'s own trigger characters for `language`, replacing that
    /// exact `(name, language)` pair's previous set: the completion-
    /// specific counterpart to `set-hook-triggers!`'s shared,
    /// listener-agnostic table (`EditorState.config.trigger_chars`), for
    /// the one reader (`orchestrate.rs`'s `Trigger::Char` arm) that needs a
    /// completion source's own answer, not a hook fire. An empty `chars`
    /// removes the `(name, language)` entry. `Err` when `name` names no
    /// registered `Buffer` source: a plugin's own typo or stale rename,
    /// not something to silently ignore the way the shared table has to
    /// (it also serves non-completion listeners, where an unregistered
    /// name isn't necessarily a mistake).
    pub(in crate::editor) fn set_buffer_trigger_chars(
        &mut self,
        name: &str,
        language: String,
        chars: Vec<char>,
    ) -> Result<(), String> {
        let id = self.buffer_id_of(name).ok_or_else(|| {
            format!("set-completion-triggers!: no buffer completion source named {name:?}")
        })?;
        let trigger_chars = &mut self.buffer[id.0 as usize].trigger_chars;
        if chars.is_empty() {
            trigger_chars.remove(&language);
        } else {
            trigger_chars.insert(language, chars);
        }
        Ok(())
    }

    /// [`Self::register_buffer`]'s `Minibuf` counterpart. Overriding a
    /// native name (`"path"`, `"command"`, …) is allowed for the same
    /// reason.
    pub(in crate::editor) fn register_minibuf(
        &mut self,
        entry: MinibufSourceEntry,
    ) -> RegisterOutcome {
        match self.minibuf_id_of(&entry.name) {
            Some(id) => {
                self.minibuf[id.0 as usize] = entry;
                RegisterOutcome::Replaced
            }
            None => {
                self.minibuf.push(entry);
                RegisterOutcome::Added
            }
        }
    }

    pub(in crate::editor) fn buffer_id_of(&self, name: &str) -> Option<BufferSourceId> {
        self.buffer
            .iter()
            .position(|e| &*e.name == name)
            .map(|i| BufferSourceId(i as u32))
    }

    pub(in crate::editor) fn minibuf_id_of(&self, name: &str) -> Option<MinibufSourceId> {
        self.minibuf
            .iter()
            .position(|e| &*e.name == name)
            .map(|i| MinibufSourceId(i as u32))
    }

    pub(in crate::editor) fn buffer_get(&self, id: BufferSourceId) -> &BufferSourceEntry {
        &self.buffer[id.0 as usize]
    }

    pub(in crate::editor) fn minibuf_get(&self, id: MinibufSourceId) -> &MinibufSourceEntry {
        &self.minibuf[id.0 as usize]
    }

    /// Every `Buffer`-target source, in registration order: what an
    /// explicit Insert-mode trigger invokes.
    pub(in crate::editor) fn buffer_sources(&self) -> Vec<BufferSourceId> {
        (0..self.buffer.len() as u32).map(BufferSourceId).collect()
    }

    /// Every `Buffer` source whose own `(language, chars)` set
    /// ([`Self::set_buffer_trigger_chars`]) includes `ch` for `language`:
    /// what a trigger-char keystroke invokes (`orchestrate.rs`'s
    /// `Trigger::Char`). `[]` with no language (a buffer with no detected
    /// language never matches anything: a completion source's own trigger
    /// chars are always server-derived, and a server attach implies a
    /// language).
    pub(in crate::editor) fn buffer_sources_for_trigger(
        &self,
        ch: char,
        language: Option<&str>,
    ) -> Vec<BufferSourceId> {
        let Some(language) = language else {
            return Vec::new();
        };
        self.buffer
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.trigger_chars
                    .get(language)
                    .is_some_and(|cs| cs.contains(&ch))
            })
            .map(|(i, _)| BufferSourceId(i as u32))
            .collect()
    }
}

#[cfg(test)]
mod tests;
