use ropey::Rope;
use std::ops::Range;

/// Ensure `rope` ends with a `\n`, appending one if it doesn't.
///
/// Every `Buffer` must end with a structural trailing newline so that the
/// cursor always has a character to sit on. This helper is the single
/// enforcement point — all constructors funnel through it.
fn ensure_trailing_newline(mut rope: Rope) -> Rope {
    if rope.len_chars() == 0 || rope.char(rope.len_chars() - 1) != '\n' {
        rope.insert_char(rope.len_chars(), '\n');
    }
    rope
}

/// The core text storage type.
///
/// `Buffer` wraps a [`ropey::Rope`], which is a balanced B-tree of Unicode
/// scalar values ("chars"). All positions exposed by this API are **char
/// offsets** — indices into the sequence of Unicode scalar values, not byte
/// offsets or grapheme-cluster indices.
///
/// Why char offsets and not bytes? Ropey's native and most stable API is
/// char-indexed. Byte indices are an implementation detail we never expose.
/// Grapheme-cluster awareness (for cursor movement) lives in `grapheme.rs`
/// and converts char offsets to grapheme boundaries on the fly.
///
/// Why an immutable-style API? `insert` and `remove` return a *new* `Buffer`
/// instead of mutating in place. Ropey clones are O(log n) in time and space
/// because the rope's B-tree nodes are reference-counted and shared between
/// the old and new version ("structural sharing"). This makes cloning cheap
/// when needed, though the primary undo mechanism is changeset inversion
/// (see `ChangeSet::invert`), not buffer snapshots.
#[derive(Debug, Clone)]
pub(crate) struct Buffer {
    rope: Rope,
}

impl Buffer {
    /// Wrap a raw `Rope` into a `Buffer`. Inverse of `into_rope`.
    ///
    /// Used by `ChangeSet::apply` to construct the result buffer after
    /// mutating the rope directly. This is the *raw* constructor — it does
    /// **not** enforce the trailing `\n` invariant so that the changeset
    /// algebra (invert/compose) remains self-consistent. The invariant is
    /// upheld at the editing-operation level: `delete_char_forward` refuses
    /// to delete the structural `\n`, so no user-facing changeset can remove it.
    pub(crate) fn from_rope(rope: Rope) -> Self {
        Self { rope }
    }

    /// Consume the buffer and return the inner `Rope`.
    ///
    /// This transfers ownership without cloning — the caller gets the rope
    /// and this `Buffer` ceases to exist. Used by `ChangeSet::apply` to
    /// mutate the rope in place (O(log n) per edit) instead of flattening
    /// to a String and rebuilding (O(n)).
    pub(crate) fn into_rope(self) -> Rope {
        self.rope
    }

    /// Create an empty buffer (contains only the structural trailing newline).
    pub(crate) fn empty() -> Self {
        Self { rope: Rope::from_str("\n") }
    }

    /// Create a buffer pre-populated with `text`.
    ///
    /// A trailing `\n` is appended if `text` does not already end with one,
    /// upholding the invariant that editing buffers always end with a newline.
    pub(crate) fn from_str(text: &str) -> Self {
        Self { rope: ensure_trailing_newline(Rope::from_str(text)) }
    }

    /// Total number of Unicode scalar values (chars) in the buffer.
    ///
    /// This is the unit used for all positions and ranges in HUME.
    pub(crate) fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Returns `true` if the buffer contains no visible content — i.e., it
    /// holds only the structural trailing newline.
    pub(crate) fn is_empty(&self) -> bool {
        debug_assert!(
            self.rope.len_chars() > 0,
            "Buffer invariant violated: len_chars() == 0 (buffer must always contain at least a trailing \\n)"
        );
        self.rope.len_chars() == 1
    }

    /// Total number of lines. A buffer always has at least one line, even when
    /// empty (the single empty line). A trailing newline adds an extra empty
    /// line — this matches how most editors count lines.
    ///
    /// For example: `"hello\nworld"` has 2 lines; `"hello\n"` has 2 lines
    /// (the second is empty).
    pub(crate) fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Returns the char offset of the first character on `line_idx` (0-based).
    ///
    /// # Panics
    /// Panics if `line_idx >= self.len_lines()`.
    pub(crate) fn line_to_char(&self, line_idx: usize) -> usize {
        self.rope.line_to_char(line_idx)
    }

    /// Returns the 0-based line number that contains char offset `char_idx`.
    ///
    /// # Panics
    /// Panics if `char_idx > self.len_chars()`.
    pub(crate) fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx)
    }

    /// Returns a slice of the buffer over the given char range.
    ///
    /// [`ropey::RopeSlice`] is a lightweight view — no allocation. It is the
    /// input type for grapheme-cluster iteration in `grapheme.rs`.
    ///
    /// # Panics
    /// Panics if `range.start > range.end` or either bound is out of range.
    pub(crate) fn slice(&self, range: Range<usize>) -> ropey::RopeSlice<'_> {
        self.rope.slice(range)
    }

    /// A slice spanning the entire buffer.
    pub(crate) fn full_slice(&self) -> ropey::RopeSlice<'_> {
        self.rope.slice(..)
    }

    /// Returns the Unicode scalar value at `char_idx`, or `None` if out of bounds.
    pub(crate) fn char_at(&self, char_idx: usize) -> Option<char> {
        if char_idx >= self.len_chars() { return None; }
        Some(self.rope.char(char_idx))
    }

    /// Returns a new buffer with `text` inserted at char offset `at`.
    ///
    /// All char offsets at or after `at` in the old buffer are shifted forward
    /// by `text.chars().count()`. Selection offsets must be updated by the
    /// caller after calling this method.
    ///
    /// # Panics
    /// Panics if `at > self.len_chars()`.
    pub(crate) fn insert(&self, at: usize, text: &str) -> Self {
        // Clone is O(log n) due to ropey's structural sharing.
        let mut rope = self.rope.clone();
        rope.insert(at, text);
        Self { rope }
    }

    /// Returns a new buffer with the char range `[from, to)` removed.
    ///
    /// All char offsets at or after `to` in the old buffer are shifted back by
    /// `to - from`. Selection offsets must be updated by the caller.
    ///
    /// # Panics
    /// Panics if `from > to` or `to > self.len_chars()`.
    pub(crate) fn remove(&self, from: usize, to: usize) -> Self {
        let mut rope = self.rope.clone();
        rope.remove(from..to);
        Self { rope }
    }

}

// Implementing `Display` gives us `.to_string()` for free via the blanket
// `impl<T: Display> ToString for T`. This is the idiomatic Rust way — an
// inherent `to_string` method would shadow that blanket impl and trigger
// the `clippy::inherent_to_string` lint.
//
// Use `.to_string()` for tests, file I/O, and display — not in hot edit paths
// (it allocates a full String from the rope).
impl std::fmt::Display for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.rope.fmt(f)
    }
}

// `PartialEq` for tests: compare the text content.
impl PartialEq for Buffer {
    fn eq(&self, other: &Self) -> bool {
        self.rope == other.rope
    }
}

impl Eq for Buffer {}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rope_is_raw() {
        // from_rope is the changeset algebra path — it does NOT enforce the
        // trailing \n so that invert/compose remain self-consistent.
        // The invariant is upheld by from_str / empty (user entry points) and
        // by the editing-operation guards (e.g. delete_char_forward is a no-op
        // on the structural \n).
        let rope = Rope::from_str("hello\n");
        let buf = Buffer::from_rope(rope);
        assert_eq!(buf.to_string(), "hello\n");
    }

    #[test]
    fn empty_buffer() {
        let buf = Buffer::empty();
        assert_eq!(buf.len_chars(), 1); // structural trailing \n
        assert_eq!(buf.len_lines(), 2); // "\n" → line 0 = "\n", line 1 = ""
        assert!(buf.is_empty());
        assert_eq!(buf.to_string(), "\n");
    }

    #[test]
    fn from_str_ascii() {
        let buf = Buffer::from_str("hello\nworld");
        assert_eq!(buf.len_chars(), 12); // "hello\nworld\n"
        assert_eq!(buf.len_lines(), 3);  // line 0, line 1, trailing empty line
        assert!(!buf.is_empty());
        assert_eq!(buf.to_string(), "hello\nworld\n");
    }

    #[test]
    fn from_str_trailing_newline() {
        // A trailing newline creates an extra empty line.
        let buf = Buffer::from_str("hello\n");
        assert_eq!(buf.len_lines(), 2);
    }

    #[test]
    fn from_str_unicode() {
        // "é" can be represented as a single char (U+00E9) or as two chars
        // (U+0065 + U+0301 combining accent). `from_str` accepts whatever Rust
        // gives us. Here we use the precomposed form — one char.
        let buf = Buffer::from_str("café");
        assert_eq!(buf.len_chars(), 5); // c a f é \n
    }

    #[test]
    fn line_to_char() {
        let buf = Buffer::from_str("hello\nworld\nfoo");
        assert_eq!(buf.line_to_char(0), 0);  // "hello" starts at 0
        assert_eq!(buf.line_to_char(1), 6);  // "world" starts after "hello\n"
        assert_eq!(buf.line_to_char(2), 12); // "foo" starts after "world\n"
    }

    #[test]
    fn char_to_line() {
        let buf = Buffer::from_str("hello\nworld\nfoo");
        assert_eq!(buf.char_to_line(0), 0);  // 'h' is on line 0
        assert_eq!(buf.char_to_line(5), 0);  // '\n' is still line 0
        assert_eq!(buf.char_to_line(6), 1);  // 'w' is on line 1
        assert_eq!(buf.char_to_line(12), 2); // 'f' is on line 2
    }

    #[test]
    fn insert_at_start() {
        let buf = Buffer::from_str("world");
        let new = buf.insert(0, "hello ");
        assert_eq!(new.to_string(), "hello world\n");
        // Original is unchanged — structural sharing.
        assert_eq!(buf.to_string(), "world\n");
    }

    #[test]
    fn insert_at_end() {
        // Insert before the trailing \n (position 5 in "hello\n").
        let buf = Buffer::from_str("hello");
        let new = buf.insert(5, " world");
        assert_eq!(new.to_string(), "hello world\n");
    }

    #[test]
    fn insert_in_middle() {
        let buf = Buffer::from_str("helo");
        let new = buf.insert(3, "l"); // "hel" + "l" + "o\n"
        assert_eq!(new.to_string(), "hello\n");
    }

    #[test]
    fn remove_whole() {
        let buf = Buffer::from_str("hello");
        let new = buf.remove(0, 5); // removes "hello", leaving "\n"
        assert_eq!(new.to_string(), "\n");
        assert!(new.is_empty());
        assert_eq!(buf.to_string(), "hello\n"); // original unchanged
    }

    #[test]
    fn remove_range() {
        let buf = Buffer::from_str("hello world");
        let new = buf.remove(5, 11); // remove " world"
        assert_eq!(new.to_string(), "hello\n");
    }

    #[test]
    fn insert_then_remove_is_identity() {
        let original = Buffer::from_str("hello world");
        let after_insert = original.insert(5, " beautiful");
        let restored = after_insert.remove(5, 15);
        assert_eq!(restored, original);
    }

    #[test]
    fn slice() {
        let buf = Buffer::from_str("hello world");
        let s: String = buf.slice(6..11).to_string();
        assert_eq!(s, "world");
    }

    #[test]
    fn equality() {
        let a = Buffer::from_str("hello");
        let b = Buffer::from_str("hello");
        let c = Buffer::from_str("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
