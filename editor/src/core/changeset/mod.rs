use crate::core::text::Text;
use crate::core::error::ApplyError;

// ── Types ────────────────────────────────────────────────────────────────────

/// A single atomic edit operation within a changeset.
///
/// A changeset decomposes any text transformation into a sequence of three
/// primitives. This is the standard Operational Transformation (OT)
/// representation used by CodeMirror, Xi-editor, and Helix.
///
/// The operations are applied sequentially against the old document:
/// - `Retain` advances through both old and new documents (1:1 mapping)
/// - `Delete` consumes from the old document only (chars vanish)
/// - `Insert` produces into the new document only (chars appear)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Operation {
    /// Skip `n` chars unchanged. Advances both old and new cursors by `n`.
    Retain(usize),
    /// Remove `n` chars from the old document. Advances old cursor by `n`,
    /// new cursor stays put.
    Delete(usize),
    /// Insert `text` into the new document. Advances new cursor by
    /// `text.chars().count()`, old cursor stays put.
    Insert(String),
}

/// Sticky-side preference when mapping a position through an insertion.
///
/// When an old-document position coincides exactly with an insertion point,
/// `Assoc` resolves the ambiguity: does the mapped position land *before*
/// or *after* the new text?
///
/// ```text
/// Old doc:  h e l | l o          (cursor at offset 3, marked with |)
///                 ↓
/// Insert "XY" at 3
///                 ↓
/// New doc:  h e l X Y l o
///           Before → 3  (cursor stays glued to what was left of it)
///           After  → 5  (cursor moves past the inserted text)
/// ```
///
/// **When you need this:** `Assoc` is only relevant when calling `map_pos`
/// to move positions that were *not* produced by the edit itself — for
/// example:
/// - **External position tracking** — LSP diagnostic ranges, bookmarks, and
///   marks live in old-doc space and must be re-anchored after every edit.
/// - **Collaborative editing** — remote cursor positions arrive in old-doc
///   space and must be mapped through locally applied changesets.
///
/// Edit operations in HUME compute new cursor positions directly from
/// `ChangeSetBuilder::new_pos()`, so they never consult `map_pos`. Undo/redo
/// uses a store-and-restore strategy (the inverse `Transaction` carries the
/// original `SelectionSet`), also without `map_pos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Assoc {
    /// Stay before inserted text ("sticky left").
    /// Use this for anchors and positions that should remain pinned to the
    /// character that was at this offset before the edit.
    #[allow(dead_code)]
    Before,
    /// Move past inserted text ("sticky right").
    /// Use this for cursors that should advance past text inserted at their
    /// position (e.g. when replaying where the user's cursor ended up).
    After,
}

/// A complete description of a document transformation.
///
/// Maps an old document of `len_before` chars to a new document of `len_after`
/// chars via a sequence of `Retain`/`Delete`/`Insert` operations. The
/// operations must exactly consume `len_before` old-document chars (via
/// `Retain` + `Delete`) and produce exactly `len_after` new-document chars
/// (via `Retain` + `Insert`).
///
/// # Normalization
///
/// Operations are always normalized: adjacent ops of the same variant are
/// merged (e.g. `Retain(3), Retain(5)` becomes `Retain(8)`), and zero-length
/// ops are omitted. This makes equality comparison meaningful and keeps
/// `compose` simple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeSet {
    ops: Vec<Operation>,
    len_before: usize,
    len_after: usize,
}

// ── push_merge helper ────────────────────────────────────────────────────────

/// Push an operation onto `ops`, merging with the last element if they are
/// the same variant. Zero-length ops are silently dropped.
///
/// This is the single normalization point used by the builder, `invert`,
/// and `compose` — every path that constructs a `Vec<Operation>` goes
/// through here to guarantee the merged/no-zeros invariant.
pub(super) fn push_merge(ops: &mut Vec<Operation>, op: Operation) {
    match op {
        Operation::Retain(0) | Operation::Delete(0) => return,
        Operation::Insert(ref s) if s.is_empty() => return,
        _ => {}
    }

    match (ops.last_mut(), &op) {
        (Some(Operation::Retain(existing)), Operation::Retain(n)) => {
            *existing += n;
        }
        (Some(Operation::Delete(existing)), Operation::Delete(n)) => {
            *existing += n;
        }
        (Some(Operation::Insert(existing)), Operation::Insert(s)) => {
            existing.push_str(s);
        }
        _ => {
            ops.push(op);
        }
    }
}

// ── compose helpers ──────────────────────────────────────────────────────────

/// How many chars does this operation "consume" from its input side?
///
/// - `Retain(n)` and `Delete(n)` consume `n` chars from the old doc.
/// - `Insert` consumes `n` chars from the intermediate doc (its char length).
///
/// This is used by `compose` to find the minimum consumption for lockstep
/// advancement.
fn op_consuming_len(op: &Operation) -> usize {
    match op {
        Operation::Retain(n) | Operation::Delete(n) => *n,
        Operation::Insert(s) => s.chars().count(),
    }
}

/// Consume `n` chars from `op` and return the remainder (or the next op
/// from the iterator if `op` is fully consumed).
///
/// For `Retain(k)` and `Delete(k)`: if `k > n`, return the same variant
/// with `k - n`; otherwise fetch the next op.
///
/// For `Insert(s)`: if `s` has more than `n` chars, return `Insert` with
/// the remaining chars (after skipping the first `n`); otherwise fetch next.
fn advance_op(
    op: Operation,
    n: usize,
    iter: &mut impl Iterator<Item = Operation>,
) -> Option<Operation> {
    let remainder = match op {
        Operation::Retain(k) if k > n => Some(Operation::Retain(k - n)),
        Operation::Delete(k) if k > n => Some(Operation::Delete(k - n)),
        Operation::Insert(s) => {
            let total = s.chars().count();
            if total > n {
                let rest: String = s.chars().skip(n).collect();
                Some(Operation::Insert(rest))
            } else {
                None
            }
        }
        _ => None, // fully consumed
    };
    remainder.or_else(|| iter.next())
}

// ── ChangeSet impl ───────────────────────────────────────────────────────────

impl ChangeSet {
    /// Returns `true` if this changeset is the identity transform — all
    /// operations are `Retain` and the document is unchanged.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.ops.iter().all(|op| matches!(op, Operation::Retain(_)))
    }

    // ── apply ────────────────────────────────────────────────────────────────

    /// Apply this changeset to `buf`, producing a new buffer.
    ///
    /// Clones the buffer's rope and mutates the clone via `Rope::remove`
    /// and `Rope::insert` — each O(log n). Retain operations are free (the
    /// chars are already in the rope). Total cost: O(k log n) for k
    /// non-retain operations, compared to the O(n) cost of flattening the
    /// rope to a `String` and rebuilding.
    ///
    /// The changeset's positions are in **old-document space**. A running
    /// `delta` translates them to the mutated rope's current coordinates,
    /// the same pattern used throughout HUME's multi-selection editing.
    ///
    /// # Errors
    ///
    /// - [`ApplyError::LengthMismatch`] if `buf.len_chars() != self.len_before`.
    /// - [`ApplyError::TrailingNewlineMissing`] if the result rope doesn't end
    ///   with `\n` (the changeset deleted the structural trailing newline).
    ///
    /// On error the original `buf` is untouched — the caller still owns it.
    pub(crate) fn apply(&self, buf: &Text) -> Result<Text, ApplyError> {
        if buf.len_chars() != self.len_before {
            return Err(ApplyError::LengthMismatch {
                buf_len: buf.len_chars(),
                expected: self.len_before,
            });
        }

        // Clone the rope (O(1) — ropey uses Arc-based tree nodes). We mutate
        // the clone so that `buf` remains valid on the error path.
        let mut rope = buf.rope().clone();

        // `delta` tracks the net char-count shift from all mutations so far.
        // Changeset positions are in old-doc space; `old_pos + delta` gives
        // the corresponding position in the mutated rope.
        let mut delta: isize = 0;
        let mut old_pos: usize = 0;

        for op in &self.ops {
            match op {
                Operation::Retain(n) => {
                    // Nothing to do — these chars are already in the rope.
                    old_pos += n;
                }
                Operation::Delete(n) => {
                    // `checked_add_signed` fails loudly in both debug and release
                    // if delta somehow drives old_pos below zero — matching the
                    // pattern used in `Selection::shift`.
                    let start = old_pos.checked_add_signed(delta)
                        .expect("changeset apply: rope position underflow");
                    rope.remove(start..start + n);
                    old_pos += n;
                    delta -= *n as isize;
                }
                Operation::Insert(s) => {
                    let pos = old_pos.checked_add_signed(delta)
                        .expect("changeset apply: rope position underflow");
                    rope.insert(pos, s);
                    delta += s.chars().count() as isize;
                }
            }
        }

        if rope.len_chars() == 0 || rope.char(rope.len_chars() - 1) != '\n' {
            return Err(ApplyError::TrailingNewlineMissing);
        }
        Ok(Text::from_rope(rope, buf.line_ending()))
    }

    // ── map_pos ──────────────────────────────────────────────────────────────

    /// Map a char position from the old document to the new document.
    ///
    /// `assoc` controls what happens when `pos` falls exactly at an insertion
    /// point: `Before` keeps the position before the inserted text, `After`
    /// moves it past. This is how cursor placement after edits is determined.
    ///
    /// Positions inside a deleted region collapse to the start of the deletion
    /// in the new document (the only sensible choice — the character is gone).
    ///
    /// # Panics
    /// Panics (debug) if `pos > self.len_before`.
    #[allow(dead_code)]
    pub(crate) fn map_pos(&self, pos: usize, assoc: Assoc) -> usize {
        debug_assert!(
            pos <= self.len_before,
            "map_pos: pos {pos} exceeds len_before {}",
            self.len_before,
        );

        let mut old = 0usize; // consumed in old doc
        let mut new = 0usize; // produced in new doc

        for op in &self.ops {
            match op {
                Operation::Retain(n) => {
                    // Retain maps old[old..old+n] → new[new..new+n] (1:1).
                    // If pos falls inside this block, it maps proportionally.
                    if pos < old + n {
                        return new + (pos - old);
                    }
                    old += n;
                    new += n;
                }
                Operation::Delete(n) => {
                    // Delete removes old[old..old+n]. Any position inside
                    // the deleted range collapses to `new` (the start of
                    // whatever follows the deletion in the new doc).
                    if pos < old + n {
                        return new;
                    }
                    old += n;
                    // new doesn't advance — deleted chars vanish.
                }
                Operation::Insert(s) => {
                    let len = s.chars().count();
                    // Insert doesn't consume old chars. If the old cursor
                    // is exactly at this insertion point, Assoc decides
                    // which side the position lands on.
                    if pos == old {
                        return match assoc {
                            Assoc::Before => new,
                            Assoc::After => new + len,
                        };
                    }
                    // pos > old: the insertion is before our position.
                    // Advance new and continue scanning.
                    new += len;
                }
            }
        }

        // Past all ops — pos is at or beyond the end of the old doc.
        new + (pos - old)
    }

    // ── touches_line ─────────────────────────────────────────────────────────

    /// Returns `true` if any `Delete` or `Insert` operation in this changeset
    /// overlaps the char range `[line_start, line_end)` of `line` in the
    /// pre-edit rope.
    ///
    /// Used by `SelectionSet::translate_in_place` to decide whether to reset
    /// `horiz` (sticky display column) on non-acting pane selections whose
    /// head resided on the edited line.
    ///
    /// `rope_pre` must be the buffer text *before* the edit (the same snapshot
    /// passed to `translate_in_place`).
    pub(crate) fn touches_line(&self, rope_pre: &ropey::Rope, line: usize) -> bool {
        let line_start = rope_pre.line_to_char(line);
        let line_end = if line + 1 < rope_pre.len_lines() {
            rope_pre.line_to_char(line + 1)
        } else {
            rope_pre.len_chars()
        };

        let mut old = 0usize;
        for op in &self.ops {
            match op {
                Operation::Retain(n) => {
                    old += n;
                }
                Operation::Delete(n) => {
                    let del_start = old;
                    let del_end = old + n;
                    if del_start < line_end && del_end > line_start {
                        return true;
                    }
                    old += n;
                }
                Operation::Insert(_) => {
                    // Insert at `old` — touches the line if the insertion
                    // point falls within [line_start, line_end).
                    if old >= line_start && old < line_end {
                        return true;
                    }
                    // old doesn't advance for Insert.
                }
            }
        }
        false
    }

    // ── invert ───────────────────────────────────────────────────────────────

    /// Produce a changeset that undoes `self`.
    ///
    /// Applying `self` to `buf` gives a new buffer; applying the inverted
    /// changeset to that new buffer gives back `buf`. This is the foundation
    /// of undo.
    ///
    /// Requires the **original** buffer (`buf`) because `Delete` operations
    /// need the actual deleted text to produce `Insert` operations in the
    /// inverse.
    ///
    /// # Panics
    /// Panics if `buf.len_chars() != self.len_before`.
    #[must_use]
    pub(crate) fn invert(&self, buf: &Text) -> ChangeSet {
        assert_eq!(
            buf.len_chars(),
            self.len_before,
            "ChangeSet::invert: buffer length {} doesn't match len_before {}",
            buf.len_chars(),
            self.len_before,
        );

        let mut inv_ops: Vec<Operation> = Vec::new();
        let mut old_pos = 0usize;

        for op in &self.ops {
            match op {
                Operation::Retain(n) => {
                    push_merge(&mut inv_ops, Operation::Retain(*n));
                    old_pos += n;
                }
                Operation::Delete(n) => {
                    // To undo a deletion, re-insert the deleted text.
                    let text = buf.slice(old_pos..old_pos + n).to_string();
                    push_merge(&mut inv_ops, Operation::Insert(text));
                    old_pos += n;
                }
                Operation::Insert(s) => {
                    // To undo an insertion, delete the same number of chars.
                    let len = s.chars().count();
                    push_merge(&mut inv_ops, Operation::Delete(len));
                    // Insert doesn't consume old chars — old_pos stays.
                }
            }
        }

        ChangeSet {
            ops: inv_ops,
            len_before: self.len_after,
            len_after: self.len_before,
        }
    }

    // ── compose ──────────────────────────────────────────────────────────────

    /// Compose two sequential changesets into one.
    ///
    /// If `self` transforms document A→B and `other` transforms B→C, then
    /// `self.compose(other)` produces a single changeset transforming A→C.
    ///
    /// This is the standard OT compose algorithm: two pointers walk through
    /// `self.ops` and `other.ops` simultaneously, consuming matching amounts
    /// from each side. The key insight is:
    ///
    /// - **A's Delete** doesn't produce anything in B, so B never sees it.
    ///   It goes straight to the output.
    /// - **B's Insert** doesn't consume anything from B (it creates new text).
    ///   It goes straight to the output.
    /// - All other combinations consume from both sides in lockstep.
    ///
    /// # Panics
    /// Panics if `self.len_after != other.len_before`.
    #[must_use]
    pub(crate) fn compose(self, other: ChangeSet) -> ChangeSet {
        assert_eq!(
            self.len_after, other.len_before,
            "compose: self.len_after ({}) != other.len_before ({})",
            self.len_after, other.len_before,
        );

        let len_before = self.len_before;
        let len_after = other.len_after;

        let mut result: Vec<Operation> = Vec::new();

        // We use partial-consumption iterators. Each "current" slot holds
        // the remainder of the operation being consumed. `into_iter()` moves
        // ops out of the vecs without cloning — `Operation` values are owned
        // directly in the cursor slots.
        let mut a_ops = self.ops.into_iter();
        let mut b_ops = other.ops.into_iter();
        let mut a_cur: Option<Operation> = a_ops.next();
        let mut b_cur: Option<Operation> = b_ops.next();

        // Each iteration moves `a_cur` and `b_cur` into a single match,
        // destructures them directly, and writes the results back. This
        // eliminates the previous `if matches! { take().expect() }` idiom —
        // ownership flows through the match arms instead of being plucked out
        // after a separate borrow-only check.
        loop {
            match (a_cur, b_cur) {
                // ── Done ─────────────────────────────────────────────────────
                (None, None) => break,

                // ── A's Delete: emit and advance A only ──────────────────────
                //
                // A removed chars from the original doc. B never saw those
                // chars, so the delete goes straight to output regardless of
                // what B is currently doing. The catch-all `b` rebinds b_cur
                // unconsumed — this correctly handles `(Delete, None)` too
                // (trailing A-deletes after B is exhausted are valid).
                (Some(Operation::Delete(n)), b) => {
                    push_merge(&mut result, Operation::Delete(n));
                    a_cur = a_ops.next();
                    b_cur = b; // put back — B wasn't involved
                }

                // ── B's Insert: emit and advance B only ──────────────────────
                //
                // B added new text that didn't exist in A's output. It goes
                // straight to output regardless of what A is doing. The Delete
                // arm above has higher priority (it comes first in the match),
                // so this arm only fires when A is not a Delete — correctly
                // matching the previous `if matches!` priority order.
                // The catch-all `a` handles `(None, Insert)` correctly too.
                (a, Some(Operation::Insert(s))) => {
                    push_merge(&mut result, Operation::Insert(s));
                    b_cur = b_ops.next();
                    a_cur = a; // put back — A wasn't involved
                }

                // ── Lockstep: both sides consume the intermediate doc ────────
                //
                // At this point we know: A is not Delete (caught above),
                // B is not Insert (caught above). Both are Some.
                // Consume `min` chars from each side, then advance both.
                (Some(a), Some(b)) => {
                    let a_len = op_consuming_len(&a);
                    let b_len = op_consuming_len(&b);
                    let min = a_len.min(b_len);

                    match (&a, &b) {
                        // Retain + Retain → Retain
                        (Operation::Retain(_), Operation::Retain(_)) => {
                            push_merge(&mut result, Operation::Retain(min));
                        }
                        // Retain + Delete → Delete
                        // (A retained chars that B then deletes.)
                        (Operation::Retain(_), Operation::Delete(_)) => {
                            push_merge(&mut result, Operation::Delete(min));
                        }
                        // Insert + Retain → Insert (first `min` chars)
                        // (A inserted text that B retains.)
                        (Operation::Insert(s), Operation::Retain(_)) => {
                            let text: String = s.chars().take(min).collect();
                            push_merge(&mut result, Operation::Insert(text));
                        }
                        // Insert + Delete → cancel
                        // (A inserted text that B immediately deletes —
                        // the text never existed from the A→C perspective.)
                        (Operation::Insert(_), Operation::Delete(_)) => {
                            // No output — they cancel out.
                        }
                        // The outer arms above guarantee A ∈ {Retain, Insert}
                        // and B ∈ {Retain, Delete}. All four combinations are
                        // handled; this arm is unreachable in correct usage.
                        _ => unreachable!(
                            "compose: unexpected op pair ({a:?}, {b:?})"
                        ),
                    }

                    // Borrows from the inner match end here. Now advance both
                    // cursors by consuming the owned `a` and `b` values.
                    // `advance_op` returns the remainder of the op (if any),
                    // or pulls the next op from the iterator.
                    a_cur = advance_op(a, min, &mut a_ops);
                    b_cur = advance_op(b, min, &mut b_ops);
                }

                // ── Invariant violation ──────────────────────────────────────
                //
                // One side still has consuming ops (Retain or Delete for B,
                // Retain or Insert for A) while the other is exhausted.
                // This means self.len_after != other.len_before, which the
                // assert at the top of compose should have caught.
                (a, b) => {
                    panic!(
                        "compose: op sequences exhausted unevenly \
                         (a_cur={a:?}, b_cur={b:?})"
                    );
                }
            }
        }

        ChangeSet {
            ops: result,
            len_before,
            len_after,
        }
    }
}


pub(crate) mod builder;
pub(crate) use builder::ChangeSetBuilder;

#[cfg(test)]
mod tests;
