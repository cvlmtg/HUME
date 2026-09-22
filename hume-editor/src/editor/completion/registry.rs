//! Every completion source the editor knows, keyed by name — the six native
//! minibuffer sources compiled in, plus whatever Steel registered via
//! `(register-completion-source! …)`. One registry for both targets: a
//! `TypedCommand.completer` names a `Minibuf` entry here, and an Insert-mode
//! trigger invokes every `Buffer` entry (or the ones registered for a
//! trigger char). Lives on `ConfigState`, so `:reload-config` rebuilds it
//! from the natives by construction and a Steel entry never outlives the
//! config that registered it.
//!
//! A source's *static* facts live here (what it targets, where its token
//! starts, how its items are scored, its priority); everything about one
//! particular invocation of it — the document it saw, the span it answered
//! for, the items — is `session.rs`'s [`super::Invocation`]. A session
//! refers back here by [`SourceId`] rather than copying any of this, so a
//! name/priority/match kind has exactly one home.

use std::ops::Range;

use steel::rvals::SteelVal;

use super::{CompletionCtx, CompletionItem, MatchKind};

/// Where a `Buffer`-target source's token starts — resolved by Rust against
/// the invocation's own snapshot *before* the source runs, so it is
/// deterministic, identical on every re-invocation, and known (for the
/// seeded filter and the menu anchor) even while the source is still
/// pending. Chosen once, at registration: the framework never guesses a
/// boundary on a source's behalf.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum BufferToken {
    /// `hume_ops::edit::word_start_before(text, head, word_chars)..head` —
    /// the identifier run before the cursor, the LSP source's choice.
    Word,
    /// `head..head` — no seeding, and accept replaces nothing before the
    /// cursor. The right choice right after a non-identifier trigger.
    Cursor,
    /// The answer's own `#:span` is authoritative; an emission without one
    /// is an error.
    Custom,
}

/// [`BufferToken`]'s minibuffer counterpart, in byte offsets of the `:`
/// line's input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum MinibufToken {
    /// The whitespace-delimited argument token the cursor is in —
    /// [`super::arg_prefix`]'s start through [`super::token_end_at`]'s
    /// whitespace stop. The framework's own command-line grammar, so a
    /// source that just enumerates a universe never has to spell it.
    Arg,
    /// The answer's own span is authoritative (`:e`'s path, `:set`'s
    /// phase-dependent token).
    Custom,
}

/// Which target a source serves, carrying that target's own token rule —
/// the pairing is a type, not two fields validated against each other, so
/// a `Buffer` source with an `Arg` token can't be built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum SourceTarget {
    Buffer(BufferToken),
    Minibuf(MinibufToken),
}

/// A native minibuffer source that enumerates a stable universe — the
/// session's own matcher narrows it, and the orchestrator supplies the
/// [`MinibufToken::Arg`] span it never has to compute.
pub(in crate::editor) type NativeUniverseFn = fn(&CompletionCtx<'_>) -> Vec<CompletionItem>;

/// A native minibuffer source whose universe *is* the live input (a
/// directory listing, a parse phase): computes its own finished, ordered
/// result and its own [`MinibufToken::Custom`] byte span, fresh every
/// attempt.
pub(in crate::editor) type NativeDelegatedFn =
    fn(&str, usize, &CompletionCtx<'_>) -> (Range<usize>, Vec<CompletionItem>);

/// How a source produces its answer. The two native shapes each imply
/// their target and token rule (see [`SourceEntry::target`]) — a native
/// source's signature *is* its contract, so there is no second field to
/// keep in agreement with it. No native `Buffer`-target shape exists yet; a
/// future buffer-words source adds its own variant here.
pub(in crate::editor) enum SourceBody {
    NativeUniverse(NativeUniverseFn),
    NativeDelegated(NativeDelegatedFn),
    /// Invoked via `EditorState::queue_steel_call` with the invocation id
    /// first — `(proc id bid prefix)` for a `Buffer` source, `(proc id
    /// input cursor)` for a `Minibuf` one — and answered, sync or async,
    /// by `(completion-emit! id …)`.
    Steel {
        proc: SteelVal,
        target: SourceTarget,
    },
}

pub(in crate::editor) struct SourceEntry {
    pub(in crate::editor) name: Box<str>,
    pub(in crate::editor) match_kind: MatchKind,
    pub(in crate::editor) priority: i64,
    pub(in crate::editor) body: SourceBody,
}

impl SourceEntry {
    pub(in crate::editor) fn target(&self) -> SourceTarget {
        match &self.body {
            SourceBody::NativeUniverse(_) => SourceTarget::Minibuf(MinibufToken::Arg),
            SourceBody::NativeDelegated(_) => SourceTarget::Minibuf(MinibufToken::Custom),
            SourceBody::Steel { target, .. } => *target,
        }
    }
}

/// Index into [`SourceRegistry`]'s entry list — stable for an entry's
/// whole life, since a re-registration under the same name overwrites the
/// slot in place rather than pushing a new one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) struct SourceId(u32);

pub(in crate::editor) struct SourceRegistry {
    entries: Vec<SourceEntry>,
}

/// What [`SourceRegistry::register`] did — the caller's own report depends
/// on which.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum RegisterOutcome {
    Added,
    Replaced,
    /// The name was already taken by a source serving the other target
    /// (its own target given here); the write was refused, the existing
    /// entry is untouched.
    TargetMismatch(SourceTarget),
}

impl SourceRegistry {
    pub(in crate::editor) fn with_defaults() -> Self {
        use super::{path, set, simple};
        let universe = |name: &str, case_sensitive, body| SourceEntry {
            name: name.into(),
            match_kind: MatchKind::String { case_sensitive },
            priority: 0,
            body: SourceBody::NativeUniverse(body),
        };
        let delegated = |name: &str, body| SourceEntry {
            name: name.into(),
            match_kind: MatchKind::Delegated,
            priority: 0,
            body: SourceBody::NativeDelegated(body),
        };
        Self {
            entries: vec![
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
    /// source. Replacing keeps the entry's [`SourceId`] stable for any
    /// session still referring to it; overriding a native name is allowed —
    /// a plugin swapping in its own `path` source is a feature, not a
    /// collision — *as long as the new entry serves the same target*. A
    /// name taken by the *other* target is refused rather than silently
    /// clobbered: a `TypedCommand.completer` naming `"path"` expects a
    /// `Minibuf` source, and a `Buffer` source quietly taking that name
    /// would break `:e` completion with no compile-time signal anywhere.
    pub(in crate::editor) fn register(&mut self, entry: SourceEntry) -> RegisterOutcome {
        match self.id_of(&entry.name) {
            Some(id) => {
                let existing_target = self.entries[id.0 as usize].target();
                if existing_target != entry.target() {
                    return RegisterOutcome::TargetMismatch(existing_target);
                }
                self.entries[id.0 as usize] = entry;
                RegisterOutcome::Replaced
            }
            None => {
                self.entries.push(entry);
                RegisterOutcome::Added
            }
        }
    }

    pub(in crate::editor) fn id_of(&self, name: &str) -> Option<SourceId> {
        self.entries
            .iter()
            .position(|e| &*e.name == name)
            .map(|i| SourceId(i as u32))
    }

    pub(in crate::editor) fn get(&self, id: SourceId) -> &SourceEntry {
        &self.entries[id.0 as usize]
    }

    /// Every `Buffer`-target source, in registration order — what an
    /// explicit Insert-mode trigger invokes.
    pub(in crate::editor) fn buffer_sources(&self) -> Vec<SourceId> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e.target(), SourceTarget::Buffer(_)))
            .map(|(i, _)| SourceId(i as u32))
            .collect()
    }
}

#[cfg(test)]
mod tests;
