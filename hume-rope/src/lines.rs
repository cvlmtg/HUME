// This module is `clippy.toml`'s named exception for `ropey::Rope`'s own
// `len_lines`/`line_to_char`/`char_to_line`: every raw call site outside
// this file routes through [`ropey_line_count`]/[`line_start_char`]/
// [`char_to_ropey_line`], or through one of this file's own line/column
// functions that resolve a line's start or count the same way.
#![allow(clippy::disallowed_methods)]

use ropey::{Rope, RopeSlice};
use unicode_segmentation::UnicodeSegmentation;

use crate::cluster::{ClusterBound, ClusterRange, ClusterStart};
use crate::column::{BufferLineCol, ByteCol, CharCol, GraphemeCol};
use crate::line::{ContentLine, ContentLineCount, RopeyLine, RopeyLineCount};
use crate::offset::{CharOffset, ExclusiveRange};

/// True if `rope` satisfies the trailing-newline invariant every HUME
/// buffer upholds by construction: empty, or ending in `'\n'`. The single
/// source of truth for that check. [`content_line_count`] asserts it
/// (a caller violating it is exactly the bug class this crate exists to
/// surface), while callers that must reject a violation at runtime instead
/// of trusting it (constructing a `BufferText`, applying a `ChangeSet`) check it
/// directly.
pub fn ends_with_newline(rope: &Rope) -> bool {
    let len = rope.len_chars();
    len == 0 || rope.char(len - 1) == '\n'
}

/// Raw ropey line count. The structural trailing `\n` every HUME buffer
/// ends with makes ropey report one line past the buffer's real content;
/// this is that raw count, phantom line included. Valid on any rope;
/// always `>= 1` (ropey defines an empty rope as having one, empty, line).
pub fn ropey_line_count(rope: &Rope) -> RopeyLineCount {
    RopeyLineCount::new(rope.len_lines())
}

/// Char offset of the first char on `line`: `ropey::Rope::line_to_char`,
/// typed. The ropey-domain counterpart to [`content_line_count`]'s own
/// typing: valid for `line` up to and including the phantom trailing line
/// (its "start" is `rope.len_chars()`, one past every real char).
pub fn line_start_char(rope: &Rope, line: RopeyLine) -> CharOffset {
    CharOffset::new(rope.line_to_char(line.index()))
}

/// [`line_start_char`]'s `RopeSlice` counterpart. `hume-rope/src/grapheme.rs`'s
/// line-relative column resolvers hold a slice, not a whole `&Rope`, and
/// route through this wrapper.
pub fn slice_line_start_char(slice: RopeSlice<'_>, line: RopeyLine) -> CharOffset {
    CharOffset::new(slice.line_to_char(line.index()))
}

/// The ropey line `char_pos` falls on: `ropey::Rope::char_to_line`, typed.
pub fn char_to_ropey_line(rope: &Rope, char_pos: CharOffset) -> RopeyLine {
    RopeyLine::new(rope.char_to_line(char_pos.index()))
}

/// Index of the last ropey line (the phantom trailing line, under the
/// trailing-newline invariant). Use when a position must stay addressable up
/// to ropey's own last line, not just the last line with real content.
pub fn last_ropey_line(rope: &Rope) -> RopeyLine {
    // ropey_line_count() is always >= 1, so this never underflows.
    RopeyLine::new(ropey_line_count(rope).get() - 1)
}

/// Number of content lines: `ropey_line_count()` minus the structural
/// trailing-`\n` line ropey counts past the buffer's real content. The
/// single source of truth for "how many lines does this buffer have" from
/// a caller's point of view: line counts shown to the user, range-checked
/// line indices.
///
/// Assumes the trailing-newline invariant (debug-asserted), which every
/// `hume_editing::BufferText` upholds it by construction. Callers that instead
/// need the last valid *ropey* line, phantom line included, want
/// [`last_ropey_line`].
pub fn content_line_count(rope: &Rope) -> ContentLineCount {
    debug_assert!(
        ends_with_newline(rope),
        "content_line_count: rope does not end with '\\n': trailing-newline invariant violated"
    );
    ContentLineCount::new(ropey_line_count(rope).get().saturating_sub(1))
}

/// Index of the last content line (`content_line_count() - 1`). `0` on an
/// empty buffer (`"\n"`, one content line: the empty first line). Callers
/// clamping a target line to stay within real content use this.
pub fn last_content_line(rope: &Rope) -> ContentLine {
    ContentLine::new(content_line_count(rope).get().saturating_sub(1))
}

/// Line tokens from `line_idx` forward, each keeping its trailing line-break
/// character(s): the tokenization line diffing needs so an `Equal` hunk
/// stays byte-comparable across the trailing-empty-line boundary (a bare
/// split on `\n` would misalign a 0-char trailing line against a 1-char
/// internal `"\n"` line by exactly one char). A slice is a view, never an
/// allocation: a caller that only inspects a token (e.g.
/// [`is_empty_line_token`]) pays nothing for one that straddles a rope
/// chunk; owned text is a conversion ([`line_token_content`]) at the
/// caller's own call site, where the copy becomes visible.
///
/// Backed by `Rope::lines()`, which splits on `\n` alone under this crate's
/// ropey config (see [`strip_line_break`]), so every token but the last is
/// `\n`-terminated. An `O(log n)` seek to `line_idx` followed by one
/// traversal of the remaining lines, instead of tokenizing (and discarding)
/// every line before it.
///
/// # Panics
/// Panics if `line_idx > ropey_line_count(rope)` (matches `Rope::line_to_char`).
pub fn line_tokens_at(rope: &Rope, line_idx: RopeyLine) -> impl Iterator<Item = RopeSlice<'_>> {
    rope.lines_at(line_idx.index())
}

/// Same tokens as [`line_tokens_at`], walking backward from `line_idx`
/// itself: the first item is line `line_idx`, then `line_idx - 1`, down to
/// line 0. The exact mirror of `line_tokens_at`'s forward-from-`line_idx`
/// shape, so neither needs a call-site `+ 1`/`- 1` correction to align them.
///
/// # Panics
/// Panics if `line_idx >= ropey_line_count(rope)`.
pub fn line_tokens_back_from(
    rope: &Rope,
    line_idx: RopeyLine,
) -> impl Iterator<Item = RopeSlice<'_>> {
    rope.lines_at(line_idx.index() + 1).reversed()
}

/// Strips the trailing line break from a line-tokenization token. `'\n'` is
/// the only break there is to strip (see the crate doc's "LF is the only
/// line break" section).
pub fn strip_line_break(line: &str) -> &str {
    line.strip_suffix('\n').unwrap_or(line)
}

/// [`strip_line_break`], truncating `buf` in place and reporting whether a
/// break was actually removed: the one signal a caller needs to tell a line
/// that ended in a break from one that didn't, without re-deriving the break
/// rule itself via a separate `ends_with('\n')`.
pub(crate) fn truncate_line_break(buf: &mut String) -> bool {
    let stripped_len = strip_line_break(buf).len();
    let had_break = stripped_len != buf.len();
    buf.truncate(stripped_len);
    had_break
}

/// Owned content of a line-tokenization token (as `Rope::lines` yields it),
/// its trailing line break stripped. One allocation, not the two a
/// `Cow::from(token)` followed by a second owned copy through
/// [`strip_line_break`] would cost on a token that straddles a rope chunk.
pub fn line_token_content(token: RopeSlice<'_>) -> String {
    let mut s = String::from(token);
    truncate_line_break(&mut s);
    s
}

/// Char offset of the first char on the line *after* `line`, or
/// `rope.len_chars()` when `line` is the last ropey line. Named for what it
/// is, not for what it might look like a shorthand for: the trailing `\n` of
/// `line` itself is [`line_break`], not `- 1` of this offset.
pub fn next_line_start(rope: &Rope, line: RopeyLine) -> CharOffset {
    let next = line.advance(1);
    if next.index() < ropey_line_count(rope).get() {
        CharOffset::new(rope.line_to_char(next.index()))
    } else {
        CharOffset::new(rope.len_chars())
    }
}

/// The first cluster of `line`. A line start is a cluster start in
/// LF-normalized text: segmentation always breaks after a `\n`.
///
/// # Panics
/// Panics if `line` is the phantom line past the structural `\n`.
pub fn line_start(rope: &Rope, line: ContentLine) -> ClusterStart {
    assert!(
        line.index() < content_line_count(rope).get(),
        "line_start: line {} is not a real content line",
        line.index()
    );
    ClusterStart::mint(CharOffset::new(rope.line_to_char(line.index())))
}

/// The boundary where `line` starts, up to and including the phantom line
/// past the structural `\n`, whose start is the text end.
pub fn ropey_line_start(rope: &Rope, line: RopeyLine) -> ClusterBound {
    ClusterBound::mint(CharOffset::new(rope.line_to_char(line.index())))
}

/// The `\n` that ends `line`: one char, its own cluster.
///
/// # Panics
/// Panics if `line` is the phantom line past the structural `\n`, which has
/// no terminator.
pub fn line_break(rope: &Rope, line: ContentLine) -> ClusterStart {
    assert!(
        line.index() < content_line_count(rope).get(),
        "line_break: line {} is not a real content line (buffer has {} content lines)",
        line.index(),
        content_line_count(rope).get()
    );
    ClusterStart::mint(line_terminator_start(rope, line.into()))
}

/// `line` with its `\n`.
pub fn line_range(rope: &Rope, line: ContentLine) -> ClusterRange {
    lines_range(rope, line, line)
}

/// The lines from `first` through `last`, each with its `\n`, in either
/// order.
pub fn lines_range(rope: &Rope, first: ContentLine, last: ContentLine) -> ClusterRange {
    let (first, last) = (first.min(last), first.max(last));
    debug_assert!(
        last.index() < content_line_count(rope).get(),
        "lines_range: line {} is not a real content line",
        last.index()
    );
    let end = ropey_line_start(rope, RopeyLine::from(last).advance(1));
    // The char before the next line's start is `last`'s `\n`, its own
    // cluster.
    ClusterRange::mint(
        line_start(rope, first),
        ClusterStart::mint(end.offset().retreat(1)),
        end,
    )
}

/// `line` without its `\n`, or `None` when the line is empty.
pub fn line_content_range(rope: &Rope, line: ContentLine) -> Option<ClusterRange> {
    ClusterRange::between(
        rope.slice(..),
        line_start(rope, line),
        line_break(rope, line).into(),
    )
}

/// Byte offset of the start of the line after `line`, or the rope's byte
/// length for the last line: the byte-domain counterpart of
/// [`next_line_start`], for the wire helpers in [`crate::position_encoding`].
pub fn next_line_start_byte(rope: &Rope, line: RopeyLine) -> usize {
    let next = line.advance(1);
    if next.index() < ropey_line_count(rope).get() {
        rope.line_to_byte(next.index())
    } else {
        rope.len_bytes()
    }
}

/// Char offset of the first non-whitespace char on `line`, or the line's
/// exclusive end if the whole line is whitespace (including empty lines,
/// where that end is `line_start`). Always within `[line_start, line_end]`.
///
/// Single source of truth for "where does leading whitespace end": the
/// editor's auto-indent-on-Enter and dedent-on-Backspace paths both consult
/// it so they agree on the boundary. Thin wrapper over [`leading_indent`] for
/// callers that don't also need the indent's display width.
pub fn leading_whitespace_end(rope: &Rope, line: ContentLine) -> ClusterStart {
    leading_indent(rope, line, 1).0
}

/// Whether `ch` is inline blank space: a space, a tab, a no-break space or an
/// ideographic space. The one definition of the blank chars a line's leading
/// whitespace and a word boundary treat as space.
pub fn is_space_char(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\u{a0}' | '\u{3000}')
}

/// [`leading_whitespace_end`], plus the leading whitespace run's display
/// width in `tab_width`, one scan instead of two. The run is measured by
/// `width::blank_advance`'s rule, the one indent guides use too.
///
/// The width is a [`BufferLineCol`]: display cells from the buffer line's
/// start, which is exactly where a leading run sits. So indent math uses
/// `shift` instead of unwrapping to a bare count at the call site.
pub fn leading_indent(
    rope: &Rope,
    line: ContentLine,
    tab_width: u8,
) -> (ClusterStart, BufferLineCol) {
    let mut display_width = BufferLineCol::new(0);
    for cluster in crate::grapheme::graphemes_at(rope.slice(..), line_start(rope, line).into()) {
        // `width` is origin-agnostic (see its own doc), so this is the
        // sanctioned `.get()` crossing into it.
        let Some(advance) =
            crate::width::blank_advance(cluster.first, display_width.get() as usize, tab_width)
        else {
            return (cluster.start(), display_width);
        };
        display_width = display_width.advance_saturating(advance as u32);
    }
    // Every content line ends in a '\n', which the loop stops on.
    (line_break(rope, line), display_width)
}

/// Char offset of `line`'s terminating `\n`, or `line`'s exclusive end when
/// it has none (the last ropey line, which by definition is unterminated).
///
/// Every ropey line but the last ends in exactly one `\n`: ropey splits on
/// LF alone here (see [`strip_line_break`]), so the terminator is always that
/// one char and needs no lookbehind to identify.
///
/// Single source of truth for the terminator rule: every caller reduces to
/// one expression over this value. The wire and motion domains still
/// disagree for an empty line by design. They differ in what they do with
/// this offset, not in how they find it.
pub(crate) fn line_terminator_start(rope: &Rope, line: RopeyLine) -> CharOffset {
    let end_excl = next_line_start(rope, line);
    if line.advance(1).index() < ropey_line_count(rope).get() {
        // `end_excl` is the start of the *next* ropey line (checked above to
        // actually exist), so the char right before it is that next line's
        // own terminator, always `'\n'`: a single codepoint, always its own
        // complete grapheme cluster, never a combining-mark hazard, so
        // stepping back one char (rather than a grapheme boundary walk) is
        // sound here.
        end_excl.retreat(1)
    } else {
        end_excl
    }
}

/// A line token (the line including its trailing `\n`, as `Rope::lines`
/// yields it) is empty when it has no content char before that terminator.
/// Whitespace-only lines are NOT empty (matching Helix semantics).
///
/// Takes the slice rather than a `&str` so a caller scanning many lines never
/// materializes one: `Cow::from(RopeSlice)` copies the whole line whenever it
/// straddles a rope leaf, which is the common case for long lines.
pub fn is_empty_line_token(token: RopeSlice<'_>) -> bool {
    match token.len_chars() {
        0 => true, // phantom trailing line
        1 => token.char(0) == '\n',
        _ => false,
    }
}

/// Returns `true` if `line` is an empty line: zero content chars before its
/// terminating `\n`. Whitespace-only lines are NOT empty (matching Helix
/// semantics).
pub fn is_empty_line(rope: &Rope, line: RopeyLine) -> bool {
    is_empty_line_token(rope.line(line.index()))
}

/// The last char offset a cursor can land on for `line`.
///
/// Returns the last char before the line's `\n`, or the `\n` itself when the
/// line is empty (no other character to sit on).
///
/// This is the motion-domain "end of line": where a cursor may land,
/// including a position sitting *on* the `\n`. [`crate::position_encoding`]'s
/// wire-domain counterpart is defined by content length instead, and the two
/// disagree by design for an empty line; do not substitute one for the other.
///
/// Content domain: `line` must be a real content line. The phantom trailing
/// line has no cursor-legal position at all (`line_start` there is
/// `rope.len_chars()`, one past every char in the buffer), so admitting it
/// here would hand a caller a position violating the buffer's own
/// `head < len_chars()` invariant. Callers that might resolve onto the
/// phantom line (a scripted `goto-location!` target) clamp to
/// [`last_content_line`] before calling this, rather than this function
/// silently answering with an illegal head.
pub fn line_content_end(rope: &Rope, line: ContentLine) -> ClusterStart {
    line_content_end_at(rope, line, line_start(rope, line))
}

/// [`line_content_end`] for a caller already holding `line`'s start.
fn line_content_end_at(rope: &Rope, line: ContentLine, start: ClusterStart) -> ClusterStart {
    let break_pos = line_break(rope, line);
    if break_pos == start {
        break_pos // empty line: cursor on the `\n` itself
    } else {
        crate::grapheme::prev_cluster(rope.slice(..), break_pos.into())
            .expect("a line with content has a cluster before its break")
    }
}

/// 0-based char column of `char_pos` within `line`: `char_pos` minus the
/// line's start offset. The char-unit sibling of
/// [`crate::grapheme::grapheme_col_in_line`] /
/// [`crate::grapheme::display_col_in_line`]; inverse of `place_char_column`'s
/// `line_start + char_col`.
pub fn char_col_in_line(rope: &Rope, line: ContentLine, char_pos: CharOffset) -> CharCol {
    let line_start = CharOffset::new(rope.line_to_char(line.index()));
    CharCol::new(char_pos.chars_since(line_start))
}

/// Place the cursor at `char_col` chars (not display columns; a tab counts 1)
/// from the start of `line`, clamped to the last content character and
/// snapped to a grapheme boundary.
///
/// For callers with no `DisplayLineMap`. Display-column placement (vertical
/// motion, selection copy) uses `DisplayLineMap::char_at_buffer_line_col` in
/// `hume-editor`.
///
/// `line` is ropey-domain because `goto-location!` can address the phantom
/// line, which holds no cluster: it places on the text's last cluster, the
/// structural `\n`.
///
/// The clamp uses the line's content end, not [`next_line_start`], so the
/// result is monotonic in `char_col` (the newline is never reachable except
/// on an empty line, where it is the content end).
pub fn place_char_column(rope: &Rope, line: RopeyLine, char_col: CharCol) -> ClusterStart {
    let Some(content_line) = line.to_content(rope) else {
        return line_break(rope, last_content_line(rope));
    };
    let line_start = line_start(rope, content_line);
    let content_end = line_content_end_at(rope, content_line, line_start);
    let line_start = line_start.offset();
    // `line_start + char_col`: `char_col` is a caller-supplied char count,
    // not yet grapheme-boundary-aligned. It may land mid-cluster, which the
    // floor below corrects. Never treat this intermediate `target` as a
    // cursor position.
    let target = CharOffset::new(line_start.index() + char_col.index());

    if target >= content_end.offset() {
        content_end
    } else {
        crate::grapheme::snap_to_cluster(rope.slice(..), target)
            .expect("target lies below the line's content end")
            .start()
    }
}

/// Place the cursor at `grapheme_col` **grapheme clusters** from the start of
/// `line` (0-based), clamping to the line's last content cluster.
///
/// The grapheme-unit sibling of [`place_char_column`], for callers whose
/// column came from somewhere a user reads it back (the statusline's
/// `line:col`, or a CLI `path:line:col` argument) rather than from an
/// addressing protocol. A char-indexed placement would land wrong on any
/// line with a multi-char grapheme (`é` = `e` + U+0301, a ZWJ emoji):
/// `place_char_column` counts each combining char as its own column, this
/// counts the whole cluster as one, matching what the caller displayed.
///
/// Same phantom-line placement as [`place_char_column`]; see its doc.
pub fn place_grapheme_column(
    rope: &Rope,
    line: RopeyLine,
    grapheme_col: GraphemeCol,
) -> ClusterStart {
    let Some(content_line) = line.to_content(rope) else {
        return line_break(rope, last_content_line(rope));
    };
    let content_end = line_content_end(rope, content_line);
    let mut pos = line_start(rope, content_line);
    for cluster in
        crate::grapheme::graphemes_at(rope.slice(..), pos.into()).take(grapheme_col.index())
    {
        if cluster.end > content_end.offset() {
            break;
        }
        pos = ClusterStart::mint(cluster.end);
    }
    pos
}

/// One buffer line's text in a reusable buffer, its `\n` stripped, with the
/// line's clusters yielded as typed positions. Segmenting a line alone gives
/// the same clusters as segmenting the whole text, because segmentation
/// always breaks around a `\n` and the text is LF-normalized.
pub struct LineText {
    buf: String,
    start: ClusterBound,
    break_pos: Option<ClusterStart>,
}

/// One cluster of a [`LineText`]: its position in the text, its bytes within
/// the line, and its text.
#[derive(Clone, Copy, Debug)]
pub struct LineCluster<'a> {
    pub start: ClusterStart,
    pub bytes: ExclusiveRange<ByteCol>,
    pub text: &'a str,
}

impl LineText {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            start: ClusterBound::TEXT_START,
            break_pos: None,
        }
    }

    /// Replace the held text with `line`'s.
    pub fn load(&mut self, rope: &Rope, line: RopeyLine) {
        self.buf.clear();
        let slice = rope.line(line.index());
        self.buf.reserve(slice.len_bytes());
        for chunk in slice.chunks() {
            self.buf.push_str(chunk);
        }
        let had_break = truncate_line_break(&mut self.buf);
        self.start = ropey_line_start(rope, line);
        self.break_pos = had_break.then(|| {
            let content_chars = slice.len_chars() - 1;
            ClusterStart::mint(CharOffset::new(self.start.offset().index() + content_chars))
        });
    }

    pub fn as_str(&self) -> &str {
        &self.buf
    }

    /// The loaded line's `\n`, when it has one.
    pub fn break_pos(&self) -> Option<ClusterStart> {
        self.break_pos
    }

    /// The loaded line's clusters, `\n` excluded, in order.
    pub fn clusters(&self) -> impl Iterator<Item = LineCluster<'_>> {
        let base = self.start.offset().index();
        self.buf
            .grapheme_indices(true)
            .scan(0usize, move |chars, (byte, text)| {
                let start = ClusterStart::mint(CharOffset::new(base + *chars));
                *chars += text.chars().count();
                let bytes =
                    ExclusiveRange::new(ByteCol::new(byte), ByteCol::new(byte + text.len()));
                Some(LineCluster { start, bytes, text })
            })
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.break_pos = None;
    }

    /// Bytes the held buffer can grow to without reallocating.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Release capacity beyond `cap` bytes.
    pub fn shrink_to(&mut self, cap: usize) {
        self.buf.shrink_to(cap);
    }
}

impl Default for LineText {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a char-offset position to a line-relative byte offset.
///
/// Returns `(line, byte_in_line)`: the byte offset from the start of
/// the line. Used to build tree-sitter `Point`s and line-relative highlight
/// spans.
pub fn char_to_line_byte(rope: &Rope, char_pos: CharOffset) -> (RopeyLine, ByteCol) {
    let char_pos = char_pos.index();
    let line = rope.char_to_line(char_pos);
    let line_start_byte = rope.line_to_byte(line);
    let byte = rope.char_to_byte(char_pos).saturating_sub(line_start_byte);
    (RopeyLine::new(line), ByteCol::new(byte))
}

/// `(row, byte_col)` of the end of `inserted` written starting at
/// `(row, byte_col)`, in tree-sitter's `Point` convention (row is a line
/// index, `byte_col` a line-relative byte offset), used to build the
/// `new_end_position` of an `InputEdit` without a second rope lookup: same
/// row with `byte_col` advanced by `inserted`'s byte length when `inserted`
/// has no `'\n'`; otherwise `row` advances by the newline count and
/// `byte_col` becomes the byte count after the last `'\n'`. Splits on
/// `'\n'` only, matching tree-sitter's own convention; callers feed
/// CRLF-normalized buffer text.
///
/// `row` is a tree-sitter `Point` row, not a rope line index: pure `'\n'`
/// counting over `inserted`, with no rope involved, so it stays `usize`
/// rather than either line-domain type.
pub fn advance_byte_point(row: usize, byte_col: ByteCol, inserted: &str) -> (usize, ByteCol) {
    match inserted.rfind('\n') {
        None => (row, byte_col.advance_saturating(inserted.len())),
        Some(last_nl) => {
            let newline_count = inserted.bytes().filter(|&b| b == b'\n').count();
            (
                row + newline_count,
                ByteCol::new(inserted.len() - last_nl - 1),
            )
        }
    }
}

/// Yield `(line, byte_start, byte_end)` for each line the *non-empty*
/// `range` char range covers at least one char of content on, clipped to
/// that line's own content (up to but excluding its trailing `\n`). Caller
/// must check `!range.is_empty()` first.
///
/// A single-line range yields one triple, byte-identical to converting
/// `range.start`/`range.end` directly with [`char_to_line_byte`]. A
/// multi-line range yields one triple per line it covers content on. The
/// clip point is deliberately the `\n` char's own position, not
/// [`next_line_start`]. The latter is the *next* line's start, which
/// `char_to_line_byte` would resolve to the wrong line (byte 0 of the line
/// after).
///
/// A range whose `start` lands exactly on a line's own `\n` (its author
/// meant "right after this line's last char", e.g. an LSP diagnostic
/// anchored at end-of-line) and continues onto the next line touches that
/// first line's *position* but covers none of its content: `seg_start` and
/// `seg_end` would both land on `line_newline` there. Skipped rather than
/// yielded zero-width: every caller flattens these into non-overlapping spans
/// downstream, and a zero-width span sorts its end before its own start at
/// the same position, which that flattening rejects by contract.
pub fn line_segments(
    rope: &Rope,
    range: ExclusiveRange<CharOffset>,
) -> impl Iterator<Item = (ContentLine, ByteCol, ByteCol)> + '_ {
    // Converts the exclusive bound to the range's own last char, only to
    // find which *line* that char is on (`char_to_line` below), never used
    // as a cursor or slice position, so landing mid-cluster (a combining
    // mark can't cross the line it's on) is harmless here. Never underflows:
    // the caller-checked `!range.is_empty()` precondition puts `range.end`
    // at `>= 1`.
    let last_char = range.end.retreat(1);
    let start_line = rope.char_to_line(range.start.index());
    let end_line = rope.char_to_line(last_char.index());
    (start_line..=end_line).filter_map(move |line_idx| {
        let line = ContentLine::new(line_idx);
        let line_newline = line_break(rope, line).offset();
        let seg_start = range
            .start
            .max(CharOffset::new(rope.line_to_char(line_idx)));
        let seg_end = range.end.min(line_newline);
        if seg_start >= seg_end {
            return None;
        }
        // Both ends are clamped to `line` above, so the line each resolves to
        // is already known. Subtracting this line's own byte offset gives the
        // same answer as `char_to_line_byte` without re-deriving the line or
        // its start byte once per end.
        let line_start_byte = rope.line_to_byte(line_idx);
        let byte_start = rope.char_to_byte(seg_start.index()) - line_start_byte;
        let byte_end = rope.char_to_byte(seg_end.index()) - line_start_byte;
        Some((line, ByteCol::new(byte_start), ByteCol::new(byte_end)))
    })
}

#[cfg(test)]
mod tests;
