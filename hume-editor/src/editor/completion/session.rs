//! The two completion session types: [`buffer::BufferSession`] (Insert mode)
//! and [`minibuf::MinibufSession`] (the `:` and `|` lines). Each stores what
//! every participating source answered and ranks it per keystroke against
//! that source's own token. `orchestrate.rs` opens sessions, invokes sources, and
//! reconciles the buffer session with its text; this module knows only what
//! a source said and where.
//!
//! Ranking and menu bookkeeping are shared in [`slots::SlotSet`], generic over
//! each target's id and span types. What differs (accepting as a buffer edit
//! vs. a splice into the minibuffer, further-typing behavior, cross-source
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

use hume_scripting::SteelCall;

pub(in crate::editor) use buffer::{BufferSession, LiveDoc, Reconciled};
pub(in crate::editor) use minibuf::MinibufSession;
pub(in crate::editor) use slots::{Invocation, ItemKey};

use super::registry::SourceRegistry;

/// A completion session as it ends, of either target.
pub(in crate::editor) trait EndingSession {
    /// The `#:cancel` calls for every request ended since the last
    /// `take_cancels`, and for every request the session still owes an
    /// answer to, which ends here.
    fn end_requests(&mut self, sources: &SourceRegistry) -> Vec<SteelCall>;
}

/// Whether `pos` sits within the closed interval `[range.start, range.end]`:
/// the completion model's own span-containment convention (a token or
/// replacement span always includes both its own endpoints, since the
/// cursor is allowed to sit exactly at either, unlike
/// `hume_rope::offset::ExclusiveRange`'s half-open one, which doesn't apply
/// here).
fn contains_cursor<T: PartialOrd>(range: &Range<T>, pos: T) -> bool {
    range.start <= pos && pos <= range.end
}

/// Boundary-safe prefix check: `str::get` returns `None` (never a panic)
/// when `prefix.len()` lands off a char boundary or past `haystack`'s end,
/// matching `complete_command`'s own original safety for non-ASCII names.
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
