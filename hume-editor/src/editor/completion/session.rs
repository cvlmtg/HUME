//! The two completion session types: [`buffer::BufferSession`] (Insert mode)
//! and [`minibuf::MinibufSession`] (the `:` line). Each stores what every
//! participating source answered and ranks it per keystroke against that
//! source's own token. `orchestrate.rs` opens sessions, invokes sources, and
//! reports edits; this module knows only what a source said and where.
//!
//! Ranking and menu bookkeeping are shared in [`slots::SlotSet`], generic over
//! each target's id and span types. What differs (accepting as a buffer edit
//! vs. a splice into the `:` line, further-typing behavior, cross-source
//! dedup) belongs to each session type, so the type itself says which target
//! a caller holds.
//!
//! Every fact about a source's answer (document snapshot, token span, items,
//! `isIncomplete`) is per-[`slots::Invocation`], not session-wide, so
//! contributors need not have seen the same buffer.

mod accept;
mod buffer;
mod minibuf;
mod slots;

use std::ops::Range;

pub(in crate::editor) use buffer::{BufferSession, LiveDoc};
pub(in crate::editor) use minibuf::MinibufSession;
pub(in crate::editor) use slots::Invocation;

/// How a source's items are matched against its token's typed text. Declared
/// per source (`registry.rs`), since one session mixes sources.
///
/// - `Fuzzy`: nucleo scoring. The candidate universe is stable (or replaced
///   by an LSP `isIncomplete` re-invocation), so [`slots::SlotSet::rank_with`]
///   re-scores locally each keystroke.
/// - `String { case_sensitive }`: a prefix gate (`starts_with`, or
///   `eq_ignore_ascii_case` on the head when not case-sensitive) with a tied
///   score of `0`. Nucleo scores any non-empty match above `0`, so once the
///   user types, `Fuzzy` items (e.g. from an LSP server) outrank `String` ones;
///   `#:priority` only breaks ties on the empty pattern.
/// - `Delegated`: the source returned a finished, ordered result. Scores are
///   tied and the sortText tiebreak is skipped, so the source's order is kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}

/// Whether `pos` sits within the closed interval `[range.start, range.end]`:
/// the completion model's own span-containment convention (a token or
/// replacement span always includes both its own endpoints, since the
/// cursor is allowed to sit exactly at either, unlike
/// `hume_rope::offset::ExclusiveRange`'s half-open one, which doesn't apply
/// here). Shared by every span-containment check in this module tree and by
/// `accept.rs`.
fn contains_cursor<T: PartialOrd>(range: &Range<T>, pos: T) -> bool {
    range.start <= pos && pos <= range.end
}

/// Boundary-safe prefix check shared by every `MatchKind::String` source:
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
