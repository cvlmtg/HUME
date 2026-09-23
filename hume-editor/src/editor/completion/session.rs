//! The two completion session types — [`buffer::BufferSession`] (Insert
//! mode) and [`minibuf::MinibufSession`] (the `:` line) — each a Rust store
//! holding what every participating source has answered, ranked per
//! keystroke against each source's *own* token. `orchestrate.rs` opens
//! whichever one a trigger names, invokes sources into it, feeds their
//! answers back, and tells a `Buffer` session about edits — this module
//! knows nothing about *how* a source runs, only what it said and where.
//!
//! Ranking and the menu are identical machinery for both targets — that
//! shared bookkeeping (a source's slot, the current ranked list, the fuzzy
//! matcher, the menu's selected row) lives in [`slots::SlotSet`], generic
//! over each target's own id and span types
//! ([`registry::BufferSourceId`](super::registry::BufferSourceId)/
//! [`buffer::BufferSpan`] vs.
//! [`registry::MinibufSourceId`](super::registry::MinibufSourceId)/
//! [`minibuf::MinibufSpan`]). What genuinely differs — the accept
//! mechanism (a buffer edit vs. a splice into the `:` line's input),
//! further-typing behavior, whether cross-source dedup applies at all —
//! is each session type's own, so there is no arm for the other target
//! that can never run, and no outside caller has to ask "which target is
//! this?" before it can read anything: a `&BufferSession` or
//! `&MinibufSession` reference already answers that by its type.
//!
//! Every fact about a source's answer is per-[`slots::Invocation`], never
//! session-wide: the document snapshot its `textEdit` ranges were computed
//! against, the token span it answered for, its items, its `isIncomplete`
//! flag. A second source, or the same source re-invoked against a later
//! document (the LSP `isIncomplete` flow), gets its own — so nothing here
//! has to assume every contributor saw the same buffer.

mod accept;
mod buffer;
mod minibuf;
mod slots;

use std::ops::Range;

pub(in crate::editor) use buffer::{BufferSession, LiveDoc};
pub(in crate::editor) use minibuf::MinibufSession;
pub(in crate::editor) use slots::Invocation;

/// How a source's items are matched against its token's typed text — a
/// per-source declaration (`registry.rs`), since one session mixes sources
/// with different universes.
///
/// - `Fuzzy` — nucleo scoring (`FuzzyMatcher`). The source's candidate
///   universe is stable (or replaced wholesale by a re-invocation, the LSP
///   `isIncomplete` flow); [`slots::SlotSet::rank_with`] re-scores it
///   locally on every keystroke without re-invoking the source.
/// - `String { case_sensitive }` — a boundary-safe prefix gate (`starts_with`,
///   or `eq_ignore_ascii_case` on the matching-length head when
///   `case_sensitive` is `false`), tied score on a match. The source's
///   universe is *also* stable (e.g. "every registered command name") — only
///   the matching rule differs from `Fuzzy`. A `String` match's tied score
///   is always `0`, never above a `Fuzzy` match's own score once anything is
///   typed (nucleo scores every non-empty match above `0`) — deliberate, not
///   a gap: in a buffer with an attached LSP server, its `Fuzzy` items should
///   win once the user narrows by typing, and a `String`-kind source (e.g.
///   `core:buffer-words`) earns its keep where `Fuzzy` sources answer
///   nothing at all (a comment, a string literal, a plain-text buffer with
///   no server) — `#:priority` only ever breaks a tie on the *empty*
///   pattern, where every source scores `0` alike. See `score_slot`'s own
///   `MatchKind::String` arm.
/// - `Delegated` — the source computed its own finished, already-ordered
///   result fresh from the live input (a directory read, a multi-phase
///   parse); this session does no scoring of its own for these items: a
///   tied score, and `rank_with`'s own sort key skips the sortText tiebreak
///   for a `Delegated` slot entirely, so the final index-ascending tiebreak
///   preserves the source's own order regardless of what `sort_text` an
///   item happens to carry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}

/// Whether `pos` sits within the closed interval `[range.start, range.end]`
/// — the completion model's own span-containment convention (a token or
/// replacement span always includes both its own endpoints, since the
/// cursor is allowed to sit exactly at either — unlike
/// `hume_rope::offset::ExclusiveRange`'s half-open one, which doesn't apply
/// here). Shared by every span-containment check in this module tree and by
/// `accept.rs`.
fn contains_cursor<T: PartialOrd>(range: &Range<T>, pos: T) -> bool {
    range.start <= pos && pos <= range.end
}

/// Boundary-safe prefix check shared by every `MatchKind::String` source —
/// `str::get` returns `None` (never a panic) when `prefix.len()` lands off a
/// char boundary or past `haystack`'s end, matching `complete_command`'s own
/// original safety for non-ASCII names.
fn prefix_matches(haystack: &str, prefix: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        haystack.starts_with(prefix)
    } else {
        haystack
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    }
}

#[cfg(test)]
mod tests;
