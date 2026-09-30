use hume_rope::offset::CharOffset;

use super::{ChangeSet, Operation, push_merge};
use crate::text::LfText;

/// Incremental builder for constructing a `ChangeSet` from positions in the
/// old document, for edits that arrive as raw char ranges (a language
/// server's text edits). Commands build their edits with
/// [`crate::edit::EditBuilder`] instead.
///
/// The builder tracks two cursors: `old_pos` (how far it has consumed in the
/// old document) and `new_pos` (how far it has produced in the new one).
/// Adjacent operations of the same kind are merged (via `push_merge`), and
/// zero-length operations are dropped.
///
/// # Usage pattern
///
/// ```text
/// let mut b = ChangeSetBuilder::new(text.end());
/// b.retain_to(start);  // keep everything before `start`
/// b.delete_to(end);    // delete `start..end`
/// b.insert("hello");   // insert the replacement
/// let cs = b.finish(); // keep everything else
/// ```
pub struct ChangeSetBuilder {
    ops: Vec<Operation>,
    doc_len: CharOffset,
    old_pos: CharOffset,
    new_pos: CharOffset,
}

impl ChangeSetBuilder {
    /// Create a builder for a document of `doc_len` chars, typically
    /// `text.end()`.
    pub fn new(doc_len: CharOffset) -> Self {
        Self {
            ops: Vec::new(),
            doc_len,
            old_pos: CharOffset::new(0),
            new_pos: CharOffset::new(0),
        }
    }

    /// Skip `n` chars unchanged.
    ///
    /// # Panics
    /// Debug-panics if `old_pos + n` would exceed `doc_len`.
    pub(crate) fn retain(&mut self, n: usize) -> &mut Self {
        debug_assert!(
            self.old_pos.shift(n as isize) <= self.doc_len,
            "ChangeSetBuilder::retain: old_pos ({:?}) + n ({n}) > doc_len ({:?})",
            self.old_pos,
            self.doc_len,
        );
        push_merge(&mut self.ops, Operation::Retain(n));
        self.old_pos = self.old_pos.shift(n as isize);
        self.new_pos = self.new_pos.shift(n as isize);
        self
    }

    /// Delete `n` chars from the old document.
    ///
    /// # Panics
    /// Debug-panics if `old_pos + n` would exceed `doc_len`.
    pub(crate) fn delete(&mut self, n: usize) -> &mut Self {
        debug_assert!(
            self.old_pos.shift(n as isize) <= self.doc_len,
            "ChangeSetBuilder::delete: old_pos ({:?}) + n ({n}) > doc_len ({:?})",
            self.old_pos,
            self.doc_len,
        );
        push_merge(&mut self.ops, Operation::Delete(n));
        self.old_pos = self.old_pos.shift(n as isize);
        // new_pos doesn't advance: deleted chars vanish.
        self
    }

    /// Insert `text` into the new document at the current position.
    ///
    /// `text` is normalized to LF here (see
    /// [`crate::text::normalize_line_endings`]). This is the one place text
    /// from outside the editing model (a pasted clipboard, a register, a
    /// language server's edit) becomes buffer content through an edit, and
    /// `ChangeSet`'s fields are private, so normalizing here is what makes
    /// "a live rope never carries a `\r`" a property of the type rather than
    /// a rule every call site has to remember. The other half of the
    /// guarantee is [`crate::text::BufferText::from`], which covers text that
    /// becomes a buffer without going through an edit at all.
    ///
    /// `new_pos` advances by the *normalized* length, so a caller reading
    /// `new_pos()` to place a cursor lands on text that actually exists.
    pub fn insert(&mut self, text: &str) -> &mut Self {
        self.insert_normalized(LfText::new(text))
    }

    /// [`insert`](Self::insert) for text already normalized, moved in without
    /// a copy.
    pub(crate) fn insert_normalized(&mut self, text: LfText) -> &mut Self {
        let len = text.as_str().chars().count();
        push_merge(&mut self.ops, Operation::Insert(text.into_string()));
        self.new_pos = self.new_pos.shift(len as isize);
        // old_pos doesn't advance: insertion doesn't consume old chars.
        self
    }

    /// Insert a single Unicode character.
    ///
    /// Convenience wrapper around [`insert`](Self::insert) that handles the
    /// `char → &str` conversion without allocating. `char` cannot be used as
    /// `&str` directly in Rust: `str` is a UTF-8 byte sequence and a `char`
    /// is a Unicode scalar value that may encode to 1–4 bytes.
    pub fn insert_char(&mut self, ch: char) -> &mut Self {
        let mut text = [0u8; 4];
        self.insert(ch.encode_utf8(&mut text))
    }

    /// Current position in the old document (chars consumed so far).
    pub(crate) fn old_pos(&self) -> CharOffset {
        self.old_pos
    }

    /// Current position in the new document (chars produced so far).
    ///
    /// After emitting an `insert`, `new_pos()` tells you exactly where a
    /// cursor should land in the result buffer.
    pub(crate) fn new_pos(&self) -> CharOffset {
        self.new_pos
    }

    /// Keep the old document up to `pos`, which must not be before what the
    /// builder has consumed.
    pub fn retain_to(&mut self, pos: CharOffset) -> &mut Self {
        self.retain(pos.chars_since(self.old_pos))
    }

    /// Delete the old document up to `pos`, which must not be before what
    /// the builder has consumed.
    pub fn delete_to(&mut self, pos: CharOffset) -> &mut Self {
        self.delete(pos.chars_since(self.old_pos))
    }

    /// Retain all remaining chars from `old_pos` to end of document.
    pub(crate) fn retain_rest(&mut self) -> &mut Self {
        let remaining = self.doc_len.chars_since(self.old_pos);
        if remaining > 0 {
            self.retain(remaining);
        }
        self
    }

    /// Keep the rest of the old document and return the finished
    /// `ChangeSet`.
    pub fn finish(mut self) -> ChangeSet {
        self.retain_rest();
        self.finish_consumed()
    }

    /// Return the finished `ChangeSet`.
    ///
    /// # Panics
    /// Panics if the builder hasn't consumed the entire old document
    /// (`old_pos != doc_len`). This catches bugs where the caller forgot
    /// to `retain_rest()`.
    pub(crate) fn finish_consumed(self) -> ChangeSet {
        assert_eq!(
            self.old_pos, self.doc_len,
            "ChangeSetBuilder::finish: old_pos ({:?}) != doc_len ({:?}). \
             Did you forget to call retain_rest()?",
            self.old_pos, self.doc_len,
        );
        ChangeSet {
            ops: self.ops,
            len_before: self.doc_len.index(),
            len_after: self.new_pos.index(),
        }
    }
}
