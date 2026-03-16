use ropey::Rope;
use std::borrow::Cow;
use std::ops::Range;

/// Whether the original file used LF or CRLF line endings.
///
/// Stored in the buffer so we can write the file back with the same endings.
/// Internally, all buffer content is normalized to LF — `\r` is never present
/// in the rope after loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineEnding {
    /// Unix / macOS (default)
    Lf,
    /// Windows / DOS
    CrLf,
}

/// Strip `\r` from `\r\n` pairs (CRLF → LF). Bare `\r` (old Mac) is left as-is.
///
/// Returns the normalized text (borrowed if no CRLF found, owned otherwise)
/// and the detected `LineEnding`. If any `\r\n` pair is present, `CrLf` is
/// returned even if some lines use LF only ("mixed" files are treated as CRLF).
fn normalize_crlf(text: &str) -> (Cow<'_, str>, LineEnding) {
    if !text.contains('\r') {
        return (Cow::Borrowed(text), LineEnding::Lf);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut found_crlf = false;
    while let Some(ch) = chars.next() {
        if ch == '\r' && chars.peek() == Some(&'\n') {
            found_crlf = true;
            // Skip the \r; the \n will be pushed on the next iteration.
            continue;
        }
        out.push(ch);
    }
    let ending = if found_crlf { LineEnding::CrLf } else { LineEnding::Lf };
    (Cow::Owned(out), ending)
}

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
    /// Original line-ending style. The rope is always LF-normalized internally;
    /// this field records what to write back on save.
    line_ending: LineEnding,
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
        // Raw constructor for ChangeSet::apply — no CRLF normalization needed
        // because the source buffer was already normalized on load.
        // The trailing-\n invariant is now enforced by ChangeSet::apply returning
        // Result before this constructor is called. This debug_assert is retained
        // as defense-in-depth for internal bugs in non-production builds.
        debug_assert!(
            rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n',
            "Buffer invariant violated: rope must end with '\\n' (len={})",
            rope.len_chars(),
        );
        Self { rope, line_ending: LineEnding::Lf }
    }

    /// Consume the buffer and return the inner `Rope`.
    ///
    /// This transfers ownership without cloning — the caller gets the rope
    /// and this `Buffer` ceases to exist.
    pub(crate) fn into_rope(self) -> Rope {
        self.rope
    }

    /// Borrow the inner `Rope`.
    ///
    /// Ropey's `Rope::clone` is O(1) (Arc-based tree), so calling
    /// `.rope().clone()` is cheap and is the preferred way to get a mutable
    /// copy for operations that take `&Buffer` instead of consuming it.
    pub(crate) fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Create an empty buffer (contains only the structural trailing newline).
    pub(crate) fn empty() -> Self {
        Self { rope: Rope::from_str("\n"), line_ending: LineEnding::Lf }
    }

    /// Create a buffer pre-populated with `text`.
    ///
    /// CRLF (`\r\n`) line endings are normalized to LF; the original style is
    /// stored in `line_ending` for use when writing back to disk. Bare `\r`
    /// (old Mac) is preserved as-is.
    ///
    /// A trailing `\n` is appended if the normalized text doesn't end with one.
    /// We check `ends_with('\n')` on the `&str` (O(1) byte check) rather than
    /// `ensure_trailing_newline` on the rope (O(log n) traversal).
    pub(crate) fn from_str(text: &str) -> Self {
        let (normalized, line_ending) = normalize_crlf(text);
        let rope = if normalized.ends_with('\n') {
            Rope::from_str(&normalized)
        } else {
            let mut r = Rope::from_str(&normalized);
            r.insert_char(r.len_chars(), '\n');
            r
        };
        Self { rope, line_ending }
    }

    /// The line-ending style of the original file.
    ///
    /// The rope is always stored with LF (`\n`) only; this records what
    /// to write back on save.
    pub(crate) fn line_ending(&self) -> LineEnding {
        self.line_ending
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
        self.rope.len_chars() == 1 && self.rope.char(0) == '\n'
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
        Self { rope, line_ending: self.line_ending }
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
        Self { rope, line_ending: self.line_ending }
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

// `PartialEq` for tests: compare text content only.
// `line_ending` is file-origin metadata — two buffers with identical content
// but different original line endings are considered equal.
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
    fn from_str_lf_line_ending() {
        let buf = Buffer::from_str("hello\n");
        assert_eq!(buf.line_ending(), LineEnding::Lf);
    }

    #[test]
    fn from_str_crlf_normalized() {
        let buf = Buffer::from_str("hello\r\nworld\r\n");
        // \r stripped — content is pure LF
        assert_eq!(buf.to_string(), "hello\nworld\n");
        assert_eq!(buf.len_chars(), 12); // "hello\nworld\n"
        assert_eq!(buf.line_ending(), LineEnding::CrLf);
    }

    #[test]
    fn from_str_mixed_crlf_lf() {
        // Mixed: CRLF wins if any \r\n present.
        let buf = Buffer::from_str("hello\r\nworld\n");
        assert_eq!(buf.to_string(), "hello\nworld\n");
        assert_eq!(buf.line_ending(), LineEnding::CrLf);
    }

    #[test]
    fn from_str_bare_cr_preserved() {
        // Old Mac bare \r is left as-is (treated as content, not a line ending).
        let buf = Buffer::from_str("hello\rworld\n");
        assert_eq!(buf.to_string(), "hello\rworld\n");
        assert_eq!(buf.line_ending(), LineEnding::Lf);
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
