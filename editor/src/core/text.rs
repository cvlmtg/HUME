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

/// The core text storage type.
///
/// `Text` wraps a [`ropey::Rope`], which is a balanced B-tree of Unicode
/// scalar values ("chars"). All positions exposed by this API are **char
/// offsets** — indices into the sequence of Unicode scalar values, not byte
/// offsets or grapheme-cluster indices.
///
/// Why char offsets and not bytes? Ropey's native and most stable API is
/// char-indexed. Byte indices are an implementation detail we never expose.
/// Grapheme-cluster awareness (for cursor movement) lives in `grapheme.rs`
/// and converts char offsets to grapheme boundaries on the fly.
///
/// Why an immutable-style API? `insert` and `remove` return a *new* `Text`
/// instead of mutating in place. Ropey clones are O(log n) in time and space
/// because the rope's B-tree nodes are reference-counted and shared between
/// the old and new version ("structural sharing"). This makes cloning cheap
/// when needed, though the primary undo mechanism is changeset inversion
/// (see `ChangeSet::invert`), not buffer snapshots.
#[derive(Debug, Clone)]
pub(crate) struct Text {
    rope: Rope,
    /// Original line-ending style. The rope is always LF-normalized internally;
    /// this field records what to write back on save.
    line_ending: LineEnding,
}

impl Text {
    /// Wrap a raw `Rope` into a `Text`. Inverse of `into_rope`.
    ///
    /// Used by `ChangeSet::apply` to construct the result buffer after
    /// mutating the rope directly. The trailing-`\n` invariant is enforced
    /// by `ChangeSet::apply` returning `Err(TrailingNewlineMissing)` before
    /// this constructor is called. The `debug_assert` here is retained as
    /// defense-in-depth for internal bugs in non-production builds.
    ///
    /// `line_ending` must be propagated from the source buffer so that CRLF
    /// metadata is preserved across edits and correctly written back on save.
    pub(crate) fn from_rope(rope: Rope, line_ending: LineEnding) -> Self {
        // Raw constructor for ChangeSet::apply — no CRLF normalization needed
        // because the source buffer was already normalized on load.
        debug_assert!(
            rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n',
            "Text invariant violated: rope must end with '\\n' (len={})",
            rope.len_chars(),
        );
        Self { rope, line_ending }
    }

    /// Borrow the inner `Rope`.
    ///
    /// Ropey's `Rope::clone` is O(log n) (reference-counted tree nodes), so
    /// calling `.rope().clone()` is cheap and is the preferred way to get a
    /// mutable copy for operations that take `&Text` instead of consuming it.
    pub(crate) fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Create an empty buffer (contains only the structural trailing newline).
    pub(crate) fn empty() -> Self {
        Self { rope: Rope::from_str("\n"), line_ending: LineEnding::Lf }
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

    /// Index of the last content character — the character just before the
    /// structural trailing `\n`.
    ///
    /// Edit operations that must not consume the trailing `\n` cap their
    /// `end_inclusive` at this value.
    pub(crate) fn last_content_char(&self) -> usize {
        self.len_chars().saturating_sub(2)
    }

    /// Returns `true` if the buffer contains no visible content — i.e., it
    /// holds only the structural trailing newline.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        debug_assert!(
            self.rope.len_chars() > 0,
            "Text invariant violated: len_chars() == 0 (buffer must always contain at least a trailing \\n)"
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

    /// Convert a byte offset to a char (Unicode scalar value) offset.
    ///
    /// Used to convert regex match byte offsets (from `regex-cursor`) back to
    /// HUME's native char-offset coordinate system. The byte offset must lie on
    /// a UTF-8 codepoint boundary; behaviour is unspecified otherwise.
    pub(crate) fn byte_to_char(&self, byte_idx: usize) -> usize {
        self.rope.byte_to_char(byte_idx)
    }

    /// Convert a char offset to a byte offset.
    ///
    /// Used to translate HUME's char-indexed cursor positions into the byte
    /// offsets that `regex-cursor` operates on.
    pub(crate) fn char_to_byte(&self, char_idx: usize) -> usize {
        self.rope.char_to_byte(char_idx)
    }

    /// Total byte length of the buffer content.
    pub(crate) fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    #[cfg(test)]
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

    #[cfg(test)]
    /// Returns a new buffer with `range` of chars removed.
    ///
    /// All char offsets at or after `range.end` in the old buffer are shifted
    /// back by `range.len()`. Selection offsets must be updated by the caller.
    ///
    /// Using `Range<usize>` (rather than two separate `from`/`to` parameters)
    /// matches ropey's own convention and makes call sites read naturally:
    /// `buf.remove(5..11)` mirrors `buf.slice(5..11)`.
    ///
    /// # Panics
    /// Panics if `range.start > range.end` or `range.end > self.len_chars()`.
    pub(crate) fn remove(&self, range: Range<usize>) -> Self {
        let mut rope = self.rope.clone();
        rope.remove(range);
        Self { rope, line_ending: self.line_ending }
    }

}

// `From<&str>` is the right trait for infallible construction from a string
// slice — as opposed to `FromStr`, which is reserved for *fallible* parsing
// (it returns `Result`). Because our construction always succeeds (worst case
// we append a '\n'), `From` is the correct choice.
//
// Implementing `From<&str>` automatically gives us:
//   - `Text::from("text")` — explicit conversion
//   - `"text".into()` where the target type is known to be `Text`
//   - Blanket `impl Into<Text> for &str` (Rust derives this from From for free)
//
// Why not an inherent `from_str` method?
//   An inherent `fn from_str(text: &str) -> Self` shadows the `FromStr` trait
//   method of the same name without actually implementing the trait, making it
//   look like a parse operation that *should* be fallible. The `From` trait
//   signals "this is an infallible type conversion", which is exactly what we
//   want here.
//
// Deref coercion note: `From` trait resolution does NOT trigger Rust's
// automatic deref coercions — `Text::from(&my_string)` won't compile
// because `&String ≠ &str` from the type-checker's perspective. Call sites
// with a `String` must be explicit: `Text::from(my_string.as_str())`.
// This is a common Rust surprise; the explicit call is also clearer to read.
impl From<&str> for Text {
    fn from(text: &str) -> Self {
        let (normalized, line_ending) = normalize_crlf(text);
        // O(1) byte check on the &str before building the rope, avoiding the
        // O(log n) rope traversal that `ensure_trailing_newline` would need.
        let rope = if normalized.ends_with('\n') {
            Rope::from_str(&normalized)
        } else {
            let mut r = Rope::from_str(&normalized);
            r.insert_char(r.len_chars(), '\n');
            r
        };
        Self { rope, line_ending }
    }
}

// Implementing `Display` gives us `.to_string()` for free via the blanket
// `impl<T: Display> ToString for T`. This is the idiomatic Rust way — an
// inherent `to_string` method would shadow that blanket impl and trigger
// the `clippy::inherent_to_string` lint.
//
// Use `.to_string()` for tests, file I/O, and display — not in hot edit paths
// (it allocates a full String from the rope).
impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.rope.fmt(f)
    }
}

// `PartialEq` for tests: compare text content only.
// `line_ending` is file-origin metadata — two buffers with identical content
// but different original line endings are considered equal.
impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.rope == other.rope
    }
}

impl Eq for Text {}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rope_is_raw() {
        // from_rope is the changeset algebra path — it has a debug_assert for
        // the trailing \n but does not add one if missing. The caller
        // (ChangeSet::apply) is responsible for ensuring the invariant holds.
        // The invariant is upheld by From<&str> / empty (user entry points) and
        // by the editing-operation guards (e.g. delete_char_forward is a no-op
        // on the structural \n).
        let rope = Rope::from_str("hello\n");
        let buf = Text::from_rope(rope, LineEnding::Lf);
        assert_eq!(buf.to_string(), "hello\n");
    }

    #[test]
    fn empty_buffer() {
        let buf = Text::empty();
        assert_eq!(buf.len_chars(), 1); // structural trailing \n
        assert_eq!(buf.len_lines(), 2); // "\n" → line 0 = "\n", line 1 = ""
        assert!(buf.is_empty());
        assert_eq!(buf.to_string(), "\n");
    }

    #[test]
    fn from_str_ascii() {
        let buf = Text::from("hello\nworld");
        assert_eq!(buf.len_chars(), 12); // "hello\nworld\n"
        assert_eq!(buf.len_lines(), 3);  // line 0, line 1, trailing empty line
        assert!(!buf.is_empty());
        assert_eq!(buf.to_string(), "hello\nworld\n");
    }

    #[test]
    fn from_str_lf_line_ending() {
        let buf = Text::from("hello\n");
        assert_eq!(buf.line_ending(), LineEnding::Lf);
    }

    #[test]
    fn from_str_crlf_normalized() {
        let buf = Text::from("hello\r\nworld\r\n");
        // \r stripped — content is pure LF
        assert_eq!(buf.to_string(), "hello\nworld\n");
        assert_eq!(buf.len_chars(), 12); // "hello\nworld\n"
        assert_eq!(buf.line_ending(), LineEnding::CrLf);
    }

    #[test]
    fn from_str_mixed_crlf_lf() {
        // Mixed: CRLF wins if any \r\n present.
        let buf = Text::from("hello\r\nworld\n");
        assert_eq!(buf.to_string(), "hello\nworld\n");
        assert_eq!(buf.line_ending(), LineEnding::CrLf);
    }

    #[test]
    fn from_str_bare_cr_preserved() {
        // Old Mac bare \r is left as-is (treated as content, not a line ending).
        let buf = Text::from("hello\rworld\n");
        assert_eq!(buf.to_string(), "hello\rworld\n");
        assert_eq!(buf.line_ending(), LineEnding::Lf);
    }

    #[test]
    fn from_str_trailing_newline() {
        // A trailing newline creates an extra empty line.
        let buf = Text::from("hello\n");
        assert_eq!(buf.len_lines(), 2);
    }

    #[test]
    fn from_str_unicode() {
        // "é" can be represented as a single char (U+00E9) or as two chars
        // (U+0065 + U+0301 combining accent). `Text::from` accepts whatever
        // Rust gives us. Here we use the precomposed form — one char.
        let buf = Text::from("café");
        assert_eq!(buf.len_chars(), 5); // c a f é \n
    }

    #[test]
    fn line_to_char() {
        let buf = Text::from("hello\nworld\nfoo");
        assert_eq!(buf.line_to_char(0), 0);  // "hello" starts at 0
        assert_eq!(buf.line_to_char(1), 6);  // "world" starts after "hello\n"
        assert_eq!(buf.line_to_char(2), 12); // "foo" starts after "world\n"
    }

    #[test]
    fn char_to_line() {
        let buf = Text::from("hello\nworld\nfoo");
        assert_eq!(buf.char_to_line(0), 0);  // 'h' is on line 0
        assert_eq!(buf.char_to_line(5), 0);  // '\n' is still line 0
        assert_eq!(buf.char_to_line(6), 1);  // 'w' is on line 1
        assert_eq!(buf.char_to_line(12), 2); // 'f' is on line 2
    }

    #[test]
    fn insert_at_start() {
        let buf = Text::from("world");
        let new = buf.insert(0, "hello ");
        assert_eq!(new.to_string(), "hello world\n");
        // Original is unchanged — structural sharing.
        assert_eq!(buf.to_string(), "world\n");
    }

    #[test]
    fn insert_at_end() {
        // Insert before the trailing \n (position 5 in "hello\n").
        let buf = Text::from("hello");
        let new = buf.insert(5, " world");
        assert_eq!(new.to_string(), "hello world\n");
    }

    #[test]
    fn insert_in_middle() {
        let buf = Text::from("helo");
        let new = buf.insert(3, "l"); // "hel" + "l" + "o\n"
        assert_eq!(new.to_string(), "hello\n");
    }

    #[test]
    fn remove_whole() {
        let buf = Text::from("hello");
        let new = buf.remove(0..5); // removes "hello", leaving "\n"
        assert_eq!(new.to_string(), "\n");
        assert!(new.is_empty());
        assert_eq!(buf.to_string(), "hello\n"); // original unchanged
    }

    #[test]
    fn remove_range() {
        let buf = Text::from("hello world");
        let new = buf.remove(5..11); // remove " world"
        assert_eq!(new.to_string(), "hello\n");
    }

    #[test]
    fn insert_then_remove_is_identity() {
        let original = Text::from("hello world");
        let after_insert = original.insert(5, " beautiful");
        let restored = after_insert.remove(5..15);
        assert_eq!(restored, original);
    }

    #[test]
    fn slice() {
        let buf = Text::from("hello world");
        let s: String = buf.slice(6..11).to_string();
        assert_eq!(s, "world");
    }

    #[test]
    fn equality() {
        let a = Text::from("hello");
        let b = Text::from("hello");
        let c = Text::from("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    // ── char_at boundary cases ────────────────────────────────────────────────

    #[test]
    fn char_at_first_position() {
        let buf = Text::from("hello");
        assert_eq!(buf.char_at(0), Some('h'));
    }

    #[test]
    fn char_at_last_position() {
        // "hello" + structural '\n' → last char is '\n' at len_chars()-1.
        let buf = Text::from("hello");
        assert_eq!(buf.char_at(buf.len_chars() - 1), Some('\n'));
    }

    #[test]
    fn char_at_out_of_bounds() {
        let buf = Text::from("hello");
        assert_eq!(buf.char_at(buf.len_chars()), None);
    }

    // ── single-char buffer ────────────────────────────────────────────────────

    #[test]
    fn single_char_buffer_has_two_chars() {
        // "x" gets the structural '\n' appended → len_chars() == 2.
        let buf = Text::from("x");
        assert_eq!(buf.len_chars(), 2);
        assert!(!buf.is_empty());
        assert_eq!(buf.char_at(0), Some('x'));
        assert_eq!(buf.char_at(1), Some('\n'));
    }

    // ── remove with empty range ───────────────────────────────────────────────

    #[test]
    fn remove_empty_range_is_identity() {
        let buf = Text::from("hello");
        let same = buf.remove(3..3);
        assert_eq!(same.to_string(), "hello\n");
    }

    // ── insert/remove with multi-byte content ─────────────────────────────────

    #[test]
    fn insert_grapheme_cluster() {
        // Insert a two-char grapheme (e + combining acute) at position 0.
        let buf = Text::from("hello");
        let new = buf.insert(0, "e\u{0301}");
        // 'e' + U+0301 + "hello" + '\n' = 8 chars.
        assert_eq!(new.len_chars(), 8);
        assert_eq!(new.char_at(0), Some('e'));
        assert_eq!(new.char_at(1), Some('\u{0301}'));
        assert_eq!(new.char_at(2), Some('h'));
    }

    #[test]
    fn remove_grapheme_cluster_range() {
        // Remove a two-char grapheme cluster.
        let buf = Text::from("e\u{0301}hello");
        // buf: 'e'(0) U+0301(1) 'h'(2) 'e'(3) ... '\n'(7) = 8 chars.
        let new = buf.remove(0..2); // remove the 'e' + combining accent
        assert_eq!(new.to_string(), "hello\n");
    }
}
