use super::{push_merge, ChangeSet, Operation};

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
/// ```text
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

    /// Insert a single Unicode character.
    ///
    /// Convenience wrapper around [`insert`](Self::insert) that handles the
    /// `char → &str` conversion without allocating. `char` cannot be used as
    /// `&str` directly in Rust: `str` is a UTF-8 byte sequence and a `char`
    /// is a Unicode scalar value that may encode to 1–4 bytes.
    pub(crate) fn insert_char(&mut self, ch: char) -> &mut Self {
        let mut buf = [0u8; 4];
        self.insert(ch.encode_utf8(&mut buf))
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

