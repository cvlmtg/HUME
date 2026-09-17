//! The minibuffer's argument-completion sources, keyed by name —
//! `TypedCommand.completer: Option<&'static str>` names an entry here
//! instead of dispatching through a closed enum, which is what lets a
//! Steel-defined typed command declare a completer at all (`registry/
//! command.rs`'s own doc records the constraint-relaxation cost of that
//! widening).
//!
//! Two native-fn shapes, one per non-`Fuzzy` `MatchKind` a minibuffer source
//! actually uses (`MatchKind::Fuzzy` has no native registrant yet — nothing
//! in the minibuffer needs incremental/async collection): a `String`-kind
//! source's universe doesn't depend on the live input at all (`ctx` alone),
//! so `update_filter`'s own generic prefix gate can narrow it; a
//! `Delegated`-kind source's universe *is* the live input (a directory
//! listing, a parse phase), so it takes `(input, cursor, ctx)` and returns
//! its own finished, already-ordered result.

use rustc_hash::FxHashMap;

use super::{CompletionCtx, CompletionItem, MatchKind};

/// A `String`- or `Fuzzy`-kind native source: enumerates its whole universe
/// once; the session's own matcher narrows it per keystroke.
type NativeFn = fn(&CompletionCtx<'_>) -> Vec<CompletionItem>;

/// A `Delegated`-kind native source: computes its own finished result fresh
/// from the live input every attempt — `(span_start, items)`.
type NativeDelegatedFn = fn(&str, usize, &CompletionCtx<'_>) -> (usize, Vec<CompletionItem>);

enum SourceKind {
    Native(NativeFn),
    NativeDelegated(NativeDelegatedFn),
}

struct CompletionSourceEntry {
    match_kind: MatchKind,
    kind: SourceKind,
}

/// One completion attempt's outcome — a `String`/`Fuzzy` source's universe
/// (for the caller to build a session from and let `update_filter` narrow),
/// or a `Delegated` source's own finished `(span_start, items)`.
pub(in crate::editor) enum SourceResult {
    Universe(Vec<CompletionItem>),
    Delegated {
        span_start: usize,
        items: Vec<CompletionItem>,
    },
}

pub(in crate::editor) struct CompletionSourceRegistry {
    sources: FxHashMap<&'static str, CompletionSourceEntry>,
}

impl CompletionSourceRegistry {
    pub(in crate::editor) fn with_defaults() -> Self {
        let mut sources = FxHashMap::default();
        sources.insert(
            super::simple::COMMAND_SOURCE,
            CompletionSourceEntry {
                match_kind: MatchKind::String {
                    case_sensitive: false,
                },
                kind: SourceKind::Native(super::simple::complete_command),
            },
        );
        sources.insert(
            super::simple::BUFFER_NAME_SOURCE,
            CompletionSourceEntry {
                match_kind: MatchKind::String {
                    case_sensitive: true,
                },
                kind: SourceKind::Native(super::simple::complete_buffer_name),
            },
        );
        sources.insert(
            super::simple::THEME_SOURCE,
            CompletionSourceEntry {
                match_kind: MatchKind::String {
                    case_sensitive: true,
                },
                kind: SourceKind::Native(super::simple::complete_theme),
            },
        );
        sources.insert(
            super::path::PATH_SOURCE,
            CompletionSourceEntry {
                match_kind: MatchKind::Delegated,
                kind: SourceKind::NativeDelegated(super::path::complete_path),
            },
        );
        sources.insert(
            super::path::PATH_DIRS_ONLY_SOURCE,
            CompletionSourceEntry {
                match_kind: MatchKind::Delegated,
                kind: SourceKind::NativeDelegated(super::path::complete_path_dirs_only),
            },
        );
        sources.insert(
            super::set::SET_SOURCE,
            CompletionSourceEntry {
                match_kind: MatchKind::Delegated,
                kind: SourceKind::NativeDelegated(super::set::complete_set),
            },
        );
        Self { sources }
    }

    /// Runs `name`'s source against `(input, cursor, ctx)`, returning `None`
    /// if `name` isn't registered (a stale name after a rename — reports at
    /// `Severity::Trace`, one level up).
    pub(in crate::editor) fn run(
        &self,
        name: &str,
        input: &str,
        cursor: usize,
        ctx: &CompletionCtx<'_>,
    ) -> Option<(MatchKind, SourceResult)> {
        let entry = self.sources.get(name)?;
        let result = match entry.kind {
            SourceKind::Native(f) => SourceResult::Universe(f(ctx)),
            SourceKind::NativeDelegated(f) => {
                let (span_start, items) = f(input, cursor, ctx);
                SourceResult::Delegated { span_start, items }
            }
        };
        Some((entry.match_kind, result))
    }
}
