use crate::buffer::Buffer;

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
fn push_merge(ops: &mut Vec<Operation>, op: Operation) {
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

// ── ChangeSet impl ───────────────────────────────────────────────────────────

impl ChangeSet {
    /// The old-document length this changeset was built for.
    pub(crate) fn len_before(&self) -> usize {
        self.len_before
    }

    /// The new-document length after applying this changeset.
    pub(crate) fn len_after(&self) -> usize {
        self.len_after
    }

    /// The raw operations (for inspection in tests).
    pub(crate) fn ops(&self) -> &[Operation] {
        &self.ops
    }

    /// Returns `true` if this changeset is the identity transform — all
    /// operations are `Retain` and the document is unchanged.
    pub(crate) fn is_empty(&self) -> bool {
        self.ops.iter().all(|op| matches!(op, Operation::Retain(_)))
    }

    // ── apply ────────────────────────────────────────────────────────────────

    /// Apply this changeset to `buf`, producing a new buffer.
    ///
    /// Consumes the buffer and mutates its rope directly via `Rope::remove`
    /// and `Rope::insert` — each O(log n). Retain operations are free (the
    /// chars are already in the rope). Total cost: O(k log n) for k
    /// non-retain operations, compared to the O(n) cost of flattening the
    /// rope to a `String` and rebuilding.
    ///
    /// The changeset's positions are in **old-document space**. A running
    /// `delta` translates them to the mutated rope's current coordinates,
    /// the same pattern used throughout HUME's multi-selection editing.
    ///
    /// # Panics
    /// Panics if `buf.len_chars() != self.len_before`.
    pub(crate) fn apply(&self, buf: Buffer) -> Buffer {
        assert_eq!(
            buf.len_chars(),
            self.len_before,
            "ChangeSet::apply: buffer length {} doesn't match len_before {}",
            buf.len_chars(),
            self.len_before,
        );

        let mut rope = buf.into_rope();

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
                    let start = (old_pos as isize + delta) as usize;
                    rope.remove(start..start + n);
                    old_pos += n;
                    delta -= *n as isize;
                }
                Operation::Insert(s) => {
                    let pos = (old_pos as isize + delta) as usize;
                    rope.insert(pos, s);
                    delta += s.chars().count() as isize;
                }
            }
        }

        Buffer::from_rope(rope)
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
    pub(crate) fn invert(&self, buf: &Buffer) -> ChangeSet {
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

        loop {
            // A's Delete goes straight through — it removes chars from the
            // original doc that B never saw. Checked first so it takes
            // priority over the B's Insert case below.
            if matches!(&a_cur, Some(Operation::Delete(_))) {
                push_merge(&mut result, a_cur.take().unwrap());
                a_cur = a_ops.next();
                continue;
            }

            // B's Insert goes straight through — it adds text that didn't
            // exist in A's output. Pulling via `take()` moves the op directly
            // into `push_merge` with no extra allocation.
            if matches!(&b_cur, Some(Operation::Insert(_))) {
                push_merge(&mut result, b_cur.take().unwrap());
                b_cur = b_ops.next();
                continue;
            }

            match (&a_cur, &b_cur) {
                (None, None) => break,

                // Both exhausted is handled above. One exhausted means the
                // changesets are inconsistent.
                (None, _) | (_, None) => {
                    panic!(
                        "compose: op sequences exhausted unevenly \
                         (a_cur={a_cur:?}, b_cur={b_cur:?})"
                    );
                }

                // Both sides have a consuming operation. Take the minimum
                // length and handle partial consumption.
                (Some(a), Some(b)) => {
                    let a_len = op_consuming_len(a);
                    let b_len = op_consuming_len(b);
                    let min = a_len.min(b_len);

                    match (a, b) {
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
                        _ => unreachable!(
                            "compose: unexpected op pair ({a:?}, {b:?})"
                        ),
                    }

                    // Advance both sides by `min`, keeping any remainder.
                    a_cur = advance_op(a_cur.take().unwrap(), min, &mut a_ops);
                    b_cur = advance_op(b_cur.take().unwrap(), min, &mut b_ops);
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

// ── ChangeSetBuilder ─────────────────────────────────────────────────────────

/// Incremental builder for constructing a `ChangeSet`.
///
/// The builder tracks two cursors: `old_pos` (how far we've consumed in the
/// old document) and `new_pos` (how far we've produced in the new document).
/// This dual tracking is the key benefit: callers can read `new_pos()` at
/// any point to know where a cursor should land in the new document — no
/// separate delta accumulator needed.
///
/// Adjacent operations of the same kind are auto-merged (via `push_merge`),
/// and zero-length operations are silently dropped.
///
/// # Usage pattern
///
/// ```ignore
/// let mut b = ChangeSetBuilder::new(buf.len_chars());
/// b.retain(5);        // skip first 5 chars
/// b.delete(3);        // delete next 3
/// b.insert("hello");  // insert replacement
/// b.retain_rest();    // keep everything else
/// let cs = b.finish();
/// ```
pub(crate) struct ChangeSetBuilder {
    ops: Vec<Operation>,
    doc_len: usize,
    old_pos: usize,
    new_pos: usize,
}

impl ChangeSetBuilder {
    /// Create a builder for a document of `doc_len` chars.
    pub(crate) fn new(doc_len: usize) -> Self {
        Self {
            ops: Vec::new(),
            doc_len,
            old_pos: 0,
            new_pos: 0,
        }
    }

    /// Skip `n` chars unchanged.
    ///
    /// # Panics
    /// Debug-panics if `old_pos + n` would exceed `doc_len`.
    pub(crate) fn retain(&mut self, n: usize) -> &mut Self {
        debug_assert!(
            self.old_pos + n <= self.doc_len,
            "ChangeSetBuilder::retain: old_pos ({}) + n ({n}) > doc_len ({})",
            self.old_pos,
            self.doc_len,
        );
        push_merge(&mut self.ops, Operation::Retain(n));
        self.old_pos += n;
        self.new_pos += n;
        self
    }

    /// Delete `n` chars from the old document.
    ///
    /// # Panics
    /// Debug-panics if `old_pos + n` would exceed `doc_len`.
    pub(crate) fn delete(&mut self, n: usize) -> &mut Self {
        debug_assert!(
            self.old_pos + n <= self.doc_len,
            "ChangeSetBuilder::delete: old_pos ({}) + n ({n}) > doc_len ({})",
            self.old_pos,
            self.doc_len,
        );
        push_merge(&mut self.ops, Operation::Delete(n));
        self.old_pos += n;
        // new_pos doesn't advance — deleted chars vanish.
        self
    }

    /// Insert `text` into the new document at the current position.
    pub(crate) fn insert(&mut self, text: &str) -> &mut Self {
        let len = text.chars().count();
        push_merge(&mut self.ops, Operation::Insert(text.to_string()));
        self.new_pos += len;
        // old_pos doesn't advance — insertion doesn't consume old chars.
        self
    }

    /// Current position in the old document (chars consumed so far).
    pub(crate) fn old_pos(&self) -> usize {
        self.old_pos
    }

    /// Current position in the new document (chars produced so far).
    ///
    /// This is the key convenience: after emitting an `insert`, `new_pos()`
    /// tells you exactly where a cursor should land in the result buffer.
    pub(crate) fn new_pos(&self) -> usize {
        self.new_pos
    }

    /// Retain all remaining chars from `old_pos` to end of document.
    /// Convenience for finishing the changeset.
    pub(crate) fn retain_rest(&mut self) -> &mut Self {
        let remaining = self.doc_len - self.old_pos;
        if remaining > 0 {
            self.retain(remaining);
        }
        self
    }

    /// Consume the builder and return the finished `ChangeSet`.
    ///
    /// # Panics
    /// Panics if the builder hasn't consumed the entire old document
    /// (`old_pos != doc_len`). This catches bugs where the caller forgot
    /// to `retain_rest()`.
    pub(crate) fn finish(self) -> ChangeSet {
        assert_eq!(
            self.old_pos, self.doc_len,
            "ChangeSetBuilder::finish: old_pos ({}) != doc_len ({}). \
             Did you forget to call retain_rest()?",
            self.old_pos, self.doc_len,
        );
        ChangeSet {
            ops: self.ops,
            len_before: self.doc_len,
            len_after: self.new_pos,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── Builder tests ────────────────────────────────────────────────────────

    #[test]
    fn builder_simple() {
        let mut b = ChangeSetBuilder::new(10);
        b.retain(3);
        b.delete(2);
        b.insert("xyz");
        b.retain_rest(); // retain remaining 5
        let cs = b.finish();

        assert_eq!(cs.len_before, 10);
        assert_eq!(cs.len_after, 11); // 10 - 2 + 3 = 11
        assert_eq!(
            cs.ops,
            vec![
                Operation::Retain(3),
                Operation::Delete(2),
                Operation::Insert("xyz".into()),
                Operation::Retain(5),
            ]
        );
    }

    #[test]
    fn builder_merges_adjacent_retains() {
        let mut b = ChangeSetBuilder::new(10);
        b.retain(3);
        b.retain(5);
        b.retain_rest();
        let cs = b.finish();

        // 3 + 5 + 2 = 10, all merged into one Retain.
        assert_eq!(cs.ops, vec![Operation::Retain(10)]);
    }

    #[test]
    fn builder_merges_adjacent_deletes() {
        let mut b = ChangeSetBuilder::new(10);
        b.delete(3);
        b.delete(2);
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(
            cs.ops,
            vec![Operation::Delete(5), Operation::Retain(5)]
        );
    }

    #[test]
    fn builder_merges_adjacent_inserts() {
        let mut b = ChangeSetBuilder::new(5);
        b.insert("ab");
        b.insert("cd");
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(
            cs.ops,
            vec![Operation::Insert("abcd".into()), Operation::Retain(5)]
        );
    }

    #[test]
    fn builder_zero_length_noop() {
        let mut b = ChangeSetBuilder::new(5);
        b.retain(0);
        b.delete(0);
        b.insert("");
        b.retain_rest();
        let cs = b.finish();

        // All zero-length ops were dropped; only the final Retain remains.
        assert_eq!(cs.ops, vec![Operation::Retain(5)]);
    }

    #[test]
    fn builder_empty_document() {
        let mut b = ChangeSetBuilder::new(0);
        b.insert("hello");
        let cs = b.finish();

        assert_eq!(cs.len_before, 0);
        assert_eq!(cs.len_after, 5);
        assert_eq!(cs.ops, vec![Operation::Insert("hello".into())]);
    }

    #[test]
    fn builder_delete_then_insert_not_merged() {
        // Delete followed by Insert is a "replace" — they must stay separate
        // so that invert and compose work correctly.
        let mut b = ChangeSetBuilder::new(5);
        b.delete(3);
        b.insert("xyz");
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(
            cs.ops,
            vec![
                Operation::Delete(3),
                Operation::Insert("xyz".into()),
                Operation::Retain(2),
            ]
        );
    }

    #[test]
    fn builder_tracks_positions() {
        let mut b = ChangeSetBuilder::new(10);
        assert_eq!(b.old_pos(), 0);
        assert_eq!(b.new_pos(), 0);

        b.retain(3);
        assert_eq!(b.old_pos(), 3);
        assert_eq!(b.new_pos(), 3);

        b.delete(2);
        assert_eq!(b.old_pos(), 5);
        assert_eq!(b.new_pos(), 3); // didn't advance

        b.insert("xyz");
        assert_eq!(b.old_pos(), 5); // didn't advance
        assert_eq!(b.new_pos(), 6);

        b.retain_rest();
        assert_eq!(b.old_pos(), 10);
        assert_eq!(b.new_pos(), 11);
    }

    #[test]
    #[should_panic(expected = "old_pos (3) != doc_len (10)")]
    fn builder_finish_panics_on_unconsumed() {
        let mut b = ChangeSetBuilder::new(10);
        b.retain(3);
        b.finish(); // should panic — 7 chars unconsumed
    }

    #[test]
    fn is_empty_for_identity() {
        let mut b = ChangeSetBuilder::new(5);
        b.retain_rest();
        assert!(b.finish().is_empty());
    }

    #[test]
    fn is_empty_false_for_real_changes() {
        let mut b = ChangeSetBuilder::new(5);
        b.delete(1);
        b.retain_rest();
        assert!(!b.finish().is_empty());
    }

    // ── apply tests ──────────────────────────────────────────────────────────

    #[test]
    fn apply_identity() {
        let buf = Buffer::from_str("hello");
        let mut b = ChangeSetBuilder::new(5);
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "hello");
    }

    #[test]
    fn apply_insert_at_start() {
        let buf = Buffer::from_str("world");
        let mut b = ChangeSetBuilder::new(5);
        b.insert("hello ");
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "hello world");
    }

    #[test]
    fn apply_insert_at_end() {
        let buf = Buffer::from_str("hello");
        let mut b = ChangeSetBuilder::new(5);
        b.retain_rest();
        b.insert(" world");
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "hello world");
    }

    #[test]
    fn apply_insert_in_middle() {
        let buf = Buffer::from_str("helo");
        let mut b = ChangeSetBuilder::new(4);
        b.retain(3);
        b.insert("l");
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "hello");
    }

    #[test]
    fn apply_delete_at_start() {
        let buf = Buffer::from_str("hello world");
        let mut b = ChangeSetBuilder::new(11);
        b.delete(6); // delete "hello "
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "world");
    }

    #[test]
    fn apply_delete_at_end() {
        let buf = Buffer::from_str("hello world");
        let mut b = ChangeSetBuilder::new(11);
        b.retain(5);
        b.delete(6); // delete " world"
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "hello");
    }

    #[test]
    fn apply_replace() {
        let buf = Buffer::from_str("hello world");
        let mut b = ChangeSetBuilder::new(11);
        b.retain(6);
        b.delete(5); // delete "world"
        b.insert("rust");
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "hello rust");
    }

    #[test]
    fn apply_multi_edit() {
        // Simulate two cursors inserting "!" at positions 0 and 6 in "hello world".
        let buf = Buffer::from_str("hello world");
        let mut b = ChangeSetBuilder::new(11);
        b.insert("!");
        b.retain(6);
        b.insert("!");
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "!hello !world");
    }

    #[test]
    fn apply_delete_entire_buffer() {
        let buf = Buffer::from_str("hello");
        let mut b = ChangeSetBuilder::new(5);
        b.delete(5);
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "");
    }

    #[test]
    fn apply_empty_buffer_insert() {
        let buf = Buffer::empty();
        let mut b = ChangeSetBuilder::new(0);
        b.insert("x");
        let cs = b.finish();

        assert_eq!(cs.apply(buf).to_string(), "x");
    }

    // ── map_pos tests ────────────────────────────────────────────────────────

    #[test]
    fn map_pos_inside_retain() {
        // Identity changeset: Retain(5). Every position maps to itself.
        let mut b = ChangeSetBuilder::new(5);
        b.retain_rest();
        let cs = b.finish();

        for i in 0..=5 {
            assert_eq!(cs.map_pos(i, Assoc::Before), i);
            assert_eq!(cs.map_pos(i, Assoc::After), i);
        }
    }

    #[test]
    fn map_pos_after_insert_at_start() {
        // Insert("xx") then Retain(5). "hello" → "xxhello".
        let mut b = ChangeSetBuilder::new(5);
        b.insert("xx");
        b.retain_rest();
        let cs = b.finish();

        // pos=0 is at the insertion point.
        assert_eq!(cs.map_pos(0, Assoc::Before), 0); // before "xx"
        assert_eq!(cs.map_pos(0, Assoc::After), 2); // after "xx"
        // pos=1 → shifted by 2.
        assert_eq!(cs.map_pos(1, Assoc::Before), 3);
        assert_eq!(cs.map_pos(5, Assoc::Before), 7); // EOF
    }

    #[test]
    fn map_pos_inside_deletion() {
        // Retain(2), Delete(3), Retain(5). "hello world" → "heworld" (wait,
        // that's only 10 chars). Let's use "helloworld" (10 chars).
        // Delete chars 2,3,4 ("llo"). Result: "heworld".
        let mut b = ChangeSetBuilder::new(10);
        b.retain(2);
        b.delete(3);
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(cs.map_pos(0, Assoc::Before), 0); // before deletion
        assert_eq!(cs.map_pos(2, Assoc::Before), 2); // at deletion start
        assert_eq!(cs.map_pos(3, Assoc::Before), 2); // inside deletion → collapse
        assert_eq!(cs.map_pos(4, Assoc::Before), 2); // inside deletion → collapse
        assert_eq!(cs.map_pos(5, Assoc::Before), 2); // right after deletion
        assert_eq!(cs.map_pos(6, Assoc::Before), 3); // shifted back by 3
    }

    #[test]
    fn map_pos_at_insert_boundary() {
        // Retain(3), Insert("XX"), Retain(2). "hello" → "helXXlo".
        let mut b = ChangeSetBuilder::new(5);
        b.retain(3);
        b.insert("XX");
        b.retain_rest();
        let cs = b.finish();

        assert_eq!(cs.map_pos(3, Assoc::Before), 3); // before "XX"
        assert_eq!(cs.map_pos(3, Assoc::After), 5); // after "XX"
        assert_eq!(cs.map_pos(4, Assoc::Before), 6); // 'l' shifted by 2
    }

    #[test]
    fn map_pos_replace_pattern() {
        // Delete(3), Insert("XY"), Retain(2). "hello" → "XYlo".
        // This is a replace of "hel" with "XY".
        let mut b = ChangeSetBuilder::new(5);
        b.delete(3);
        b.insert("XY");
        b.retain_rest();
        let cs = b.finish();

        // pos=0: inside deletion → collapses to 0 (before "XY")
        assert_eq!(cs.map_pos(0, Assoc::Before), 0);
        // pos=2: inside deletion → collapses to 0
        assert_eq!(cs.map_pos(2, Assoc::Before), 0);
        // pos=3: just after deletion, at insert point.
        // Delete consumed 3, so old=3 after Delete. Insert at old=3.
        // pos==old → Assoc applies.
        assert_eq!(cs.map_pos(3, Assoc::Before), 0); // before "XY"
        assert_eq!(cs.map_pos(3, Assoc::After), 2); // after "XY"
        // pos=4: in the final Retain. old=3, new=2 after insert.
        // pos < old + 2 → new + (4-3) = 3.
        assert_eq!(cs.map_pos(4, Assoc::Before), 3);
    }

    #[test]
    fn map_pos_eof() {
        // Retain(3), Insert("XX"). "abc" → "abcXX".
        let mut b = ChangeSetBuilder::new(3);
        b.retain_rest();
        b.insert("XX");
        let cs = b.finish();

        // pos=3 (EOF) is at the insertion point.
        assert_eq!(cs.map_pos(3, Assoc::Before), 3);
        assert_eq!(cs.map_pos(3, Assoc::After), 5);
    }

    // ── invert tests ─────────────────────────────────────────────────────────

    #[test]
    fn invert_identity() {
        let buf = Buffer::from_str("hello");
        let mut b = ChangeSetBuilder::new(5);
        b.retain_rest();
        let cs = b.finish();
        let inv = cs.invert(&buf);

        assert!(inv.is_empty());
        assert_eq!(inv.len_before, 5);
        assert_eq!(inv.len_after, 5);
    }

    #[test]
    fn invert_insert() {
        // Insert "XX" at start of "hello" → "XXhello".
        // Inverse should delete 2 chars at start.
        let buf = Buffer::from_str("hello");
        let mut b = ChangeSetBuilder::new(5);
        b.insert("XX");
        b.retain_rest();
        let cs = b.finish();
        let inv = cs.invert(&buf);

        assert_eq!(inv.len_before, 7); // "XXhello"
        assert_eq!(inv.len_after, 5); // back to "hello"
        assert_eq!(
            inv.ops,
            vec![Operation::Delete(2), Operation::Retain(5)]
        );
    }

    #[test]
    fn invert_delete() {
        // Delete first 3 chars of "hello" → "lo".
        // Inverse should insert "hel" at start.
        let buf = Buffer::from_str("hello");
        let mut b = ChangeSetBuilder::new(5);
        b.delete(3);
        b.retain_rest();
        let cs = b.finish();
        let inv = cs.invert(&buf);

        assert_eq!(inv.len_before, 2); // "lo"
        assert_eq!(inv.len_after, 5); // back to "hello"
        assert_eq!(
            inv.ops,
            vec![Operation::Insert("hel".into()), Operation::Retain(2)]
        );
    }

    #[test]
    fn invert_roundtrip() {
        // Invert before apply — apply consumes the buffer.
        let buf = Buffer::from_str("hello world");
        let mut b = ChangeSetBuilder::new(11);
        b.retain(6);
        b.delete(5);
        b.insert("rust");
        let cs = b.finish();

        let inv = cs.invert(&buf);
        let result = cs.apply(buf);
        assert_eq!(result.to_string(), "hello rust");

        let restored = inv.apply(result);
        assert_eq!(restored.to_string(), "hello world");
    }

    #[test]
    fn invert_replace() {
        let buf = Buffer::from_str("abcde");
        let mut b = ChangeSetBuilder::new(5);
        b.retain(1);
        b.delete(3); // delete "bcd"
        b.insert("XY"); // insert "XY"
        b.retain_rest();
        let cs = b.finish();

        let inv = cs.invert(&buf);
        let result = cs.apply(buf);
        assert_eq!(result.to_string(), "aXYe");

        let restored = inv.apply(result);
        assert_eq!(restored.to_string(), "abcde");
    }

    #[test]
    fn invert_multi_edit() {
        // Two inserts at different positions.
        let buf = Buffer::from_str("hello world");
        let mut b = ChangeSetBuilder::new(11);
        b.insert("!");
        b.retain(6);
        b.insert("!");
        b.retain_rest();
        let cs = b.finish();

        let inv = cs.invert(&buf);
        let result = cs.apply(buf);
        assert_eq!(result.to_string(), "!hello !world");

        let restored = inv.apply(result);
        assert_eq!(restored.to_string(), "hello world");
    }

    // ── compose tests ────────────────────────────────────────────────────────

    #[test]
    fn compose_identity_left() {
        // identity ∘ cs = cs
        let mut id_b = ChangeSetBuilder::new(5);
        id_b.retain_rest();
        let id = id_b.finish();

        let mut cs_b = ChangeSetBuilder::new(5);
        cs_b.retain(2);
        cs_b.insert("X");
        cs_b.retain_rest();
        let cs = cs_b.finish();

        // cs is PartialEq — clone it so we can compare after compose consumes it.
        let composed = id.compose(cs.clone());
        assert_eq!(composed, cs);
        assert_eq!(composed.len_before, 5);
        assert_eq!(composed.len_after, 6);
    }

    #[test]
    fn compose_identity_right() {
        // cs ∘ identity = cs
        let mut cs_b = ChangeSetBuilder::new(5);
        cs_b.retain(2);
        cs_b.insert("X");
        cs_b.retain_rest();
        let cs = cs_b.finish();

        let mut id_b = ChangeSetBuilder::new(6); // len_after of cs
        id_b.retain_rest();
        let id = id_b.finish();

        let composed = cs.clone().compose(id);
        assert_eq!(composed, cs);
    }

    #[test]
    fn compose_two_inserts() {
        // A: insert "X" at 0 in "abc" → "Xabc" (len 3→4)
        // B: insert "Y" at 2 in "Xabc" → "XaYbc" (len 4→5)
        // Composed: "abc" → "XaYbc"
        let buf = Buffer::from_str("abc");

        let mut a_b = ChangeSetBuilder::new(3);
        a_b.insert("X");
        a_b.retain_rest();
        let a = a_b.finish();

        let mut b_b = ChangeSetBuilder::new(4);
        b_b.retain(2);
        b_b.insert("Y");
        b_b.retain_rest();
        let b = b_b.finish();

        // Step-by-step oracle: apply a then b separately.
        let step_by_step = b.clone().apply(a.clone().apply(buf.clone()));
        let composed = a.compose(b);
        let direct = composed.apply(buf);
        assert_eq!(direct.to_string(), step_by_step.to_string());
        assert_eq!(direct.to_string(), "XaYbc");
    }

    #[test]
    fn compose_insert_then_delete() {
        // A: insert "XY" at 0 in "abc" → "XYabc" (len 3→5)
        // B: delete 2 at 0 in "XYabc" → "abc" (len 5→3)
        // Composed: identity on "abc"
        let buf = Buffer::from_str("abc");

        let mut a_b = ChangeSetBuilder::new(3);
        a_b.insert("XY");
        a_b.retain_rest();
        let a = a_b.finish();

        let mut b_b = ChangeSetBuilder::new(5);
        b_b.delete(2);
        b_b.retain_rest();
        let b = b_b.finish();

        let composed = a.compose(b);
        assert!(composed.is_empty(), "insert then delete should cancel");
        assert_eq!(composed.apply(buf).to_string(), "abc");
    }

    #[test]
    fn compose_delete_then_insert() {
        // A: delete 3 at start of "hello" → "lo" (len 5→2)
        // B: insert "XY" at 0 in "lo" → "XYlo" (len 2→4)
        // Composed: "hello" → "XYlo"
        let buf = Buffer::from_str("hello");

        let mut a_b = ChangeSetBuilder::new(5);
        a_b.delete(3);
        a_b.retain_rest();
        let a = a_b.finish();

        let mut b_b = ChangeSetBuilder::new(2);
        b_b.insert("XY");
        b_b.retain_rest();
        let b = b_b.finish();

        let step_by_step = b.clone().apply(a.clone().apply(buf.clone()));
        let composed = a.compose(b);
        let direct = composed.apply(buf);
        assert_eq!(direct.to_string(), step_by_step.to_string());
        assert_eq!(direct.to_string(), "XYlo");
    }

    #[test]
    fn compose_complex() {
        // A: retain 2, delete 1, insert "XY", retain 2 on "abcde"
        //    → "abXYde" (5→6)
        // B: retain 1, delete 3, retain 2 on "abXYde"
        //    → "ade"... wait: delete 3 from pos 1 = "bXY", then retain "de"
        //    → "ade" (6→3)
        // Composed: "abcde" → "ade"
        let buf = Buffer::from_str("abcde");

        let mut a_b = ChangeSetBuilder::new(5);
        a_b.retain(2);
        a_b.delete(1);
        a_b.insert("XY");
        a_b.retain_rest();
        let a = a_b.finish();

        let mut b_b = ChangeSetBuilder::new(6);
        b_b.retain(1);
        b_b.delete(3);
        b_b.retain_rest();
        let b = b_b.finish();

        let step_by_step = b.clone().apply(a.clone().apply(buf.clone()));
        let composed = a.compose(b);
        let direct = composed.apply(buf);
        assert_eq!(direct.to_string(), step_by_step.to_string());
        assert_eq!(direct.to_string(), "ade");
    }

    #[test]
    fn compose_partial_insert_retain() {
        // A: insert "ABCD" at start, retain 3 on "xyz" → "ABCDxyz" (3→7)
        // B: retain 2, delete 2, retain 3 on "ABCDxyz"
        //    → "AB" + "xyz" = "ABxyz" (7→5)
        // Composed: "xyz" → "ABxyz"
        let buf = Buffer::from_str("xyz");

        let mut a_b = ChangeSetBuilder::new(3);
        a_b.insert("ABCD");
        a_b.retain_rest();
        let a = a_b.finish();

        let mut b_b = ChangeSetBuilder::new(7);
        b_b.retain(2);
        b_b.delete(2);
        b_b.retain_rest();
        let b = b_b.finish();

        let step_by_step = b.clone().apply(a.clone().apply(buf.clone()));
        let composed = a.compose(b);
        let direct = composed.apply(buf);
        assert_eq!(direct.to_string(), step_by_step.to_string());
        assert_eq!(direct.to_string(), "ABxyz");
    }

    // ── Property-based tests (proptest) ─────────────────────────────────────

    use proptest::prelude::*;

    /// Generate a random ASCII string of length 0..=max_len.
    fn arb_text(max_len: usize) -> impl Strategy<Value = String> {
        proptest::collection::vec(b'a'..=b'z', 0..=max_len)
            .prop_map(|bytes| String::from_utf8(bytes).unwrap())
    }

    /// Generate a random valid `ChangeSet` for a document of `doc_len` chars.
    ///
    /// Strategy: partition the old document into segments, each assigned a
    /// random operation (retain or delete). Insert random text between any
    /// two segments with some probability.
    fn arb_changeset(doc_len: usize) -> impl Strategy<Value = ChangeSet> {
        // Generate a sequence of (action, length) pairs that consume
        // exactly doc_len old chars, plus random inserts.
        let max_ops = (doc_len + 1).min(8); // keep it bounded
        proptest::collection::vec(
            (
                prop_oneof![Just(0u8), Just(1u8), Just(2u8)], // 0=retain, 1=delete, 2=insert
                1..=5usize,                                    // segment length
                arb_text(4),                                   // text for inserts
            ),
            1..=max_ops,
        )
        .prop_map(move |raw_ops| {
            let mut builder = ChangeSetBuilder::new(doc_len);
            let mut remaining = doc_len;

            for (action, len, text) in raw_ops {
                if remaining == 0 {
                    // Only inserts are possible once we've consumed all old chars.
                    if action == 2 && !text.is_empty() {
                        builder.insert(&text);
                    }
                    continue;
                }

                let n = len.min(remaining);

                match action {
                    0 => {
                        builder.retain(n);
                        remaining -= n;
                    }
                    1 => {
                        builder.delete(n);
                        remaining -= n;
                    }
                    2 => {
                        if !text.is_empty() {
                            builder.insert(&text);
                        }
                        // Don't consume old chars for insert.
                    }
                    _ => unreachable!(),
                }
            }

            // Retain any remaining old chars to finish the changeset.
            if remaining > 0 {
                builder.retain(remaining);
            }
            builder.finish()
        })
    }

    proptest! {
        /// Applying a changeset then its inverse restores the original buffer.
        #[test]
        fn prop_invert_roundtrip(text in arb_text(20)) {
            let doc_len = text.len();
            let buf = Buffer::from_str(&text);
            let original_text = text.clone();

            let half = doc_len / 2;
            let mut b = ChangeSetBuilder::new(doc_len);
            b.delete(half);
            b.insert("X");
            b.retain_rest();
            let cs = b.finish();

            // Invert before apply — apply consumes the buffer.
            let inv = cs.invert(&buf);
            let result = cs.apply(buf);
            let restored = inv.apply(result);
            prop_assert_eq!(restored.to_string(), original_text);
        }

        /// Composing two changesets produces the same result as applying them
        /// sequentially.
        #[test]
        fn prop_compose_equivalence(text in arb_text(20)) {
            let doc_len = text.len();
            let buf = Buffer::from_str(&text);

            // First changeset: delete first quarter, insert "AB".
            let q1 = doc_len / 4;
            let mut b1 = ChangeSetBuilder::new(doc_len);
            b1.delete(q1);
            b1.insert("AB");
            b1.retain_rest();
            let cs1 = b1.finish();

            let mid = cs1.apply(buf.clone());
            let mid_len = mid.len_chars();

            // Second changeset: retain half, insert "CD", retain rest.
            let half = mid_len / 2;
            let mut b2 = ChangeSetBuilder::new(mid_len);
            b2.retain(half);
            b2.insert("CD");
            b2.retain_rest();
            let cs2 = b2.finish();

            let step_by_step = cs2.clone().apply(mid);
            let composed = cs1.compose(cs2);
            let direct = composed.apply(buf);

            prop_assert_eq!(direct.to_string(), step_by_step.to_string());
        }

        /// Applying a random changeset then its inverse always restores the
        /// original buffer.
        #[test]
        fn prop_random_changeset_invert(
            _text in arb_text(30),
            cs in arb_text(30).prop_flat_map(|t| {
                let len = t.len();
                arb_changeset(len).prop_map(move |cs| (t.clone(), cs))
            })
        ) {
            let (text, cs) = cs;
            let buf = Buffer::from_str(&text);

            // Invert before apply — apply consumes the buffer.
            let inv = cs.invert(&buf);
            let result = cs.apply(buf);
            let restored = inv.apply(result);
            prop_assert_eq!(restored.to_string(), text);
        }
    }
}
