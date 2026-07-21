use ropey::Rope;
use std::borrow::Cow;
use std::ops::Range;

/// Whether the original file used LF or CRLF line endings.
///
/// Stored in the buffer so we can write the file back with the same endings.
/// Internally, all buffer content is normalized to LF — `\r` is never present
/// in the rope after loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
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
    let ending = if found_crlf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
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
pub struct Text {
    rope: Rope,
    /// Original line-ending style. The rope is always LF-normalized internally;
    /// this field records what to write back on save.
    line_ending: LineEnding,
}

impl Text {
    /// Wrap a raw `Rope` into a `Text`.
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
    ///
    /// # Design note
    /// This exposes the `ropey` type directly. Callers (regex search, syntax
    /// highlighting, scroll logic) need raw `Rope` / `RopeSlice` access for
    /// performance, so the boundary is intentionally permeable here. `ropey`
    /// is a stable, semver-pinned dependency; changing it would require
    /// touching the caller sites regardless.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Create an empty buffer (contains only the structural trailing newline).
    pub fn empty() -> Self {
        Self {
            rope: Rope::from_str("\n"),
            line_ending: LineEnding::Lf,
        }
    }

    /// The line-ending style of the original file.
    ///
    /// The rope is always stored with LF (`\n`) only; this records what
    /// to write back on save.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Total number of Unicode scalar values (chars) in the buffer.
    ///
    /// This is the unit used for all positions and ranges in HUME.
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Index of the last content character — the character just before the
    /// structural trailing `\n`.
    ///
    /// Edit operations that must not consume the trailing `\n` cap their
    /// `end_inclusive` at this value.
    ///
    /// Degenerate case: on an empty buffer (`"\n"`, one char) this returns 0,
    /// which is the structural `\n` itself — there is no content character to
    /// point at. Callers deleting up to this index must handle the empty
    /// buffer first, or the delete would consume the structural newline.
    pub fn last_content_char(&self) -> usize {
        self.len_chars().saturating_sub(2)
    }

    /// Returns `true` if the buffer contains no visible content — i.e., it
    /// holds only the structural trailing newline.
    #[cfg(test)]
    fn is_empty(&self) -> bool {
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
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Returns the char offset of the first character on `line_idx` (0-based).
    ///
    /// # Panics
    /// Panics if `line_idx >= self.len_lines()`.
    pub fn line_to_char(&self, line_idx: usize) -> usize {
        self.rope.line_to_char(line_idx)
    }

    /// Returns the 0-based line number that contains char offset `char_idx`.
    ///
    /// # Panics
    /// Panics if `char_idx > self.len_chars()`.
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx)
    }

    /// Returns a slice of the buffer over the given char range.
    ///
    /// [`ropey::RopeSlice`] is a lightweight view — no allocation. It is the
    /// input type for grapheme-cluster iteration in `grapheme.rs`.
    ///
    /// # Panics
    /// Panics if `range.start > range.end` or either bound is out of range.
    pub fn slice(&self, range: Range<usize>) -> ropey::RopeSlice<'_> {
        self.rope.slice(range)
    }

    /// A slice spanning the entire buffer.
    pub fn full_slice(&self) -> ropey::RopeSlice<'_> {
        self.rope.slice(..)
    }

    /// Returns the Unicode scalar value at `char_idx`, or `None` if out of bounds.
    pub fn char_at(&self, char_idx: usize) -> Option<char> {
        if char_idx >= self.len_chars() {
            return None;
        }
        Some(self.rope.char(char_idx))
    }

    /// A cursor over chars starting at `pos`, for scanning a contiguous range
    /// without re-paying ropey's O(log n) tree descent on every step (unlike
    /// repeated `char_at` calls in a loop). See [`CharCursor`].
    ///
    /// # Panics
    /// Panics if `pos > self.len_chars()`.
    pub fn chars_at(&self, pos: usize) -> CharCursor<'_> {
        CharCursor {
            iter: self.rope.chars_at(pos),
            pos,
        }
    }

    /// Convert a byte offset to a char (Unicode scalar value) offset.
    ///
    /// Used to convert regex match byte offsets (from `regex-cursor`) back to
    /// HUME's native char-offset coordinate system. The byte offset must lie on
    /// a UTF-8 codepoint boundary; behaviour is unspecified otherwise.
    pub fn byte_to_char(&self, byte_idx: usize) -> usize {
        self.rope.byte_to_char(byte_idx)
    }

    /// Convert a char offset to a byte offset.
    ///
    /// Used to translate HUME's char-indexed cursor positions into the byte
    /// offsets that `regex-cursor` operates on.
    pub fn char_to_byte(&self, char_idx: usize) -> usize {
        self.rope.char_to_byte(char_idx)
    }

    /// Total byte length of the buffer content.
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Collect the full text into a contiguous `Vec<u8>`.
    ///
    /// Walks rope chunks directly into a pre-sized `Vec` to avoid the
    /// intermediate `String` allocation that `to_string().into_bytes()` would
    /// produce (peak 2× buffer size vs 1×).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rope.len_bytes());
        for chunk in self.rope.chunks() {
            out.extend_from_slice(chunk.as_bytes());
        }
        out
    }

    /// Returns a new buffer with `text` inserted at char offset `at`.
    ///
    /// All char offsets at or after `at` in the old buffer are shifted forward
    /// by `text.chars().count()`. Selection offsets must be updated by the
    /// caller after calling this method.
    ///
    /// # Panics
    /// Panics if `at > self.len_chars()`.
    #[cfg(test)]
    fn insert(&self, at: usize, text: &str) -> Self {
        // Clone is O(log n) due to ropey's structural sharing.
        let mut rope = self.rope.clone();
        rope.insert(at, text);
        Self {
            rope,
            line_ending: self.line_ending,
        }
    }

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
    #[cfg(test)]
    fn remove(&self, range: Range<usize>) -> Self {
        let mut rope = self.rope.clone();
        rope.remove(range);
        Self {
            rope,
            line_ending: self.line_ending,
        }
    }
}

/// A char-level cursor for scanning a contiguous range of a [`Text`] without
/// re-paying ropey's O(log n) tree descent on every step. Each `next()` /
/// `prev()` call is amortized O(1) after the initial O(log n) seek in
/// [`Text::chars_at`] — an O(span × log n) loop of `char_at(i)` calls becomes
/// O(log n + span).
///
/// **Char-level, not grapheme-level** — intended for ASCII delimiter scanning
/// (brackets, quotes, argument commas), same exposure class as `char_at`.
/// Motion and selection logic must keep using `grapheme.rs` boundary helpers;
/// a multi-codepoint cluster (e.g. `e` + U+0301) is yielded here as two
/// separate chars, just like two `char_at` calls would see it.
pub struct CharCursor<'a> {
    iter: ropey::iter::Chars<'a>,
    /// Char index of the position the cursor currently sits at — the index
    /// `next()` would yield and `prev()` would land on.
    pos: usize,
}

impl Iterator for CharCursor<'_> {
    type Item = (usize, char);

    /// Yield the char at the cursor position, then advance forward.
    fn next(&mut self) -> Option<(usize, char)> {
        let ch = self.iter.next()?;
        let pos = self.pos;
        self.pos += 1;
        Some((pos, ch))
    }
}

impl CharCursor<'_> {
    /// Step back and yield the char just before the cursor position.
    ///
    /// Not a [`DoubleEndedIterator`](std::iter::DoubleEndedIterator) impl —
    /// that trait means "consume from the far end of the same forward
    /// sequence," not "walk backward from here," which is what callers
    /// (bracket-pair scans) actually need.
    pub fn prev(&mut self) -> Option<(usize, char)> {
        let ch = self.iter.prev()?;
        self.pos -= 1;
        Some((self.pos, ch))
    }
}

// `From<&str>`, not `FromStr`, since construction here always succeeds
// (worst case we append a '\n') — `FromStr` is reserved for fallible parsing.
// Note `Text::from(&my_string)` won't compile (`&String != &str`); call sites
// with a `String` must be explicit: `Text::from(my_string.as_str())`.
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
mod tests;
