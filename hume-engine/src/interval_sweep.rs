//! Event-sweep flattener for overlapping same-scan-unit spans — shared by
//! `hume-treesitter`'s injection-layer flattening (nested grammar layers,
//! `R` = nesting depth) and `hume-editor`'s diagnostic/extra-highlight
//! flattening (`R` = `std::cmp::Reverse<u8>` priority — see
//! [`flatten_overlapping_spans`]'s doc for why the wrapper).

/// Which span wins when two spans have exactly equal `rank` and overlap —
/// see [`flatten_overlapping_spans`]. The two current callers genuinely
/// disagree, so this is a real two-case distinction, not a speculative
/// flag: `hume-treesitter` wants the most-recently-collected (nested)
/// layer to win a tie; `hume-editor` wants the alphabetically-first source
/// (already sorted ascending into `raw` before the call) to win — pinned by
/// `overlapping_extra_highlights_from_two_sources_resolve_alphabetically`.
#[derive(Clone, Copy)]
pub enum TieBreak {
    /// The span pushed to `raw` first wins (`hume-editor`'s convention).
    FirstPushed,
    /// The span pushed to `raw` last wins (`hume-treesitter`'s convention).
    LastPushed,
}

/// Flattens overlapping `(start, end, rank, scope)` spans (all sharing one
/// scan unit — one line, one byte range, whatever the caller means by
/// "overlapping") into the sorted, non-overlapping `(start, end, scope)`
/// sequence a single rendering layer's contract requires: its own output
/// must not overlap itself. At each position, the span with the
/// highest-`R`-per-`Ord` wins (`R` need not be a priority number directly —
/// `hume-editor`'s caller passes `std::cmp::Reverse<u8>` so its "lower
/// priority number wins" convention becomes "highest `Reverse` value wins"
/// with no inversion arithmetic); a rank tie resolves per `tie_break`.
///
/// `raw` need not be pre-sorted; drained (left empty) on return. `stack`/
/// `events` are caller-owned scratch, reused across calls to avoid a fresh
/// allocation per call on a hot path (`hume-treesitter`'s highlighter runs
/// this per visible line, every frame) — both must be empty on entry, and
/// are empty again on return. Adjacent output segments sharing one scope
/// are merged into one — they can arise when an overlapping span ends while
/// another of the same scope is still active (e.g. A=[0,5), B=[3,8): at pos
/// 5, A ends and B continues, producing (3,5,B) then (5,8,B) without this
/// merge pass).
pub fn flatten_overlapping_spans<R: Ord + Copy, S: PartialEq + Copy>(
    raw: &mut Vec<(usize, usize, R, S)>,
    stack: &mut Vec<(R, u32, S)>,
    events: &mut Vec<(usize, bool, u32, R, S)>,
    out: &mut Vec<(usize, usize, S)>,
    tie_break: TieBreak,
) {
    debug_assert!(stack.is_empty());
    debug_assert!(events.is_empty());
    if raw.is_empty() {
        return;
    }

    // Build a sorted event list: (pos, is_end, seq, rank, scope). `seq`
    // orders a rank tie — the span's index within `raw`, or its mirror
    // image (`u32::MAX - index`) under `TieBreak::FirstPushed`, so that the
    // fixed "highest seq wins" rule below (shared with the rank ordering)
    // reads as "first pushed wins" from the caller's perspective without a
    // second, divergent comparison path. Unique either way — end events
    // pop the exact matching start by this same value, never ambiguous,
    // unlike matching by scope value when two active spans share a scope.
    // End events sort before start events at the same position so a
    // closing span is popped before a new one is pushed at the same byte.
    for (index, &(start, end, rank, scope)) in raw.iter().enumerate() {
        let seq = match tie_break {
            TieBreak::LastPushed => index as u32,
            TieBreak::FirstPushed => u32::MAX - index as u32,
        };
        events.push((start, false, seq, rank, scope));
        events.push((end, true, seq, rank, scope));
    }
    raw.clear();
    // Sort purely by (pos, ends-before-starts) — priority among
    // simultaneously active spans is resolved by the sorted-stack insertion
    // below, not by event processing order.
    events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut pos = 0usize;
    for &(event_pos, is_end, seq, rank, scope) in events.iter() {
        // Emit the gap before this event using the currently active
        // (highest rank, then highest seq) scope.
        if let Some(&(_, _, active_scope)) = stack.last()
            && pos < event_pos
        {
            out.push((pos, event_pos, active_scope));
        }
        pos = event_pos;

        if is_end {
            let idx = stack.iter().position(|&(r, s, _)| r == rank && s == seq);
            debug_assert!(
                idx.is_some(),
                "end event with no matching start on the stack — a zero-width \
                 span would sort its end before its own start at the same \
                 position; callers must filter those out before collection"
            );
            if let Some(idx) = idx {
                stack.remove(idx);
            }
        } else {
            // Insert in ascending (rank, seq) order so `stack.last()` stays
            // the highest-ranked active span regardless of arrival order.
            let insert_at = stack.partition_point(|&(r, s, _)| (r, s) < (rank, seq));
            stack.insert(insert_at, (rank, seq, scope));
        }
    }
    stack.clear();
    events.clear();

    out.dedup_by(|next, prev| {
        if prev.2 == next.2 && prev.1 == next.0 {
            prev.1 = next.1; // extend the retained segment
            true
        } else {
            false
        }
    });
}

#[cfg(test)]
mod tests;
