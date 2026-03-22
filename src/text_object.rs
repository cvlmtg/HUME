use crate::buffer::Buffer;
use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::helpers::{classify_char, is_word_boundary, is_WORD_boundary, line_end_exclusive, CharClass};
use crate::selection::{Selection, SelectionSet};

// ── Text object framework ──────────────────────────────────────────────────────

/// Apply a text object to every selection in the set.
///
/// Unlike motions, which map a single cursor position to a new position, a
/// text object maps a cursor position to a *range* — the region to select.
/// `text_object` returns `Some((start, end))` as an inclusive char-offset pair,
/// or `None` if no match exists (e.g., cursor not inside any bracket pair).
///
/// On `None`, the existing selection is preserved (Helix behaviour: `mi(` when
/// not inside parens is a no-op). On `Some`, the selection is replaced with a
/// forward selection anchored at `start` and with head at `end`.
///
/// Uses `map_and_merge` so that multiple cursors landing on the same range
/// (e.g., both cursors inside the same bracket pair) are automatically merged.
pub(crate) fn apply_text_object(
    buf: &Buffer,
    sels: SelectionSet,
    text_object: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    sels.map_and_merge(|sel| match text_object(buf, sel.head) {
        Some((start, end)) => Selection::new(start, end),
        None => sel,
    })
}

// ── Line ───────────────────────────────────────────────────────────────────────

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    // end_excl points one past the last char on this line.
    // We want the inclusive end, which is one before end_excl.
    // Then we want to exclude the trailing '\n', so one more step back.
    // If the line is "\n" only, start == end_excl - 1, so there's no
    // content before the newline — return None.
    if end_excl == start {
        // Shouldn't happen given the buffer invariant, but be safe.
        return None;
    }
    let last = end_excl - 1; // inclusive end of the raw line (the '\n' itself)
    if buf.char_at(last) == Some('\n') {
        if last == start {
            // Only char on this line is '\n' — no content.
            return None;
        }
        Some((start, last - 1))
    } else {
        // Last line without a trailing '\n' (shouldn't happen due to buffer
        // invariant, but handle gracefully).
        Some((start, last))
    }
}

/// Around line: the full line including the trailing newline.
fn around_line(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    if end_excl == start {
        return None;
    }
    Some((start, end_excl - 1))
}

pub(crate) fn cmd_inner_line(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_text_object(&buf, sels, inner_line);
    (buf, new_sels)
}

pub(crate) fn cmd_around_line(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_text_object(&buf, sels, around_line);
    (buf, new_sels)
}

// ── Word / WORD ────────────────────────────────────────────────────────────────

/// Inner word parameterised by boundary predicate.
///
/// Scans left and right from `pos` while adjacent chars share the same
/// "class" (no boundary crossing). Whatever class the char at `pos` belongs
/// to defines the selected run — including whitespace runs and EOL.
fn inner_word_impl(
    buf: &Buffer,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> Option<(usize, usize)> {
    let class = classify_char(buf.char_at(pos)?);

    // Scan left: walk back by grapheme cluster boundaries while the preceding
    // grapheme belongs to the same class. Using prev_grapheme_boundary ensures
    // we always inspect the *base* codepoint of each grapheme (not a combining
    // codepoint like U+0301 that would be misclassified as Punctuation).
    let mut start = pos;
    while start > 0 {
        let prev_pos = prev_grapheme_boundary(buf, start);
        let prev = classify_char(buf.char_at(prev_pos)?);
        if is_boundary(prev, class) {
            break;
        }
        start = prev_pos;
    }

    // Scan right: walk forward by grapheme cluster boundaries while the next
    // grapheme belongs to the same class. We track the grapheme-*start* position
    // and convert to an inclusive char-level end at the very end, so that the
    // returned range covers the full grapheme (including combining codepoints).
    let mut end_grapheme_start = pos;
    loop {
        let next_pos = next_grapheme_boundary(buf, end_grapheme_start);
        if next_pos >= buf.len_chars() {
            break;
        }
        let next = classify_char(buf.char_at(next_pos)?);
        if is_boundary(class, next) {
            break;
        }
        end_grapheme_start = next_pos;
    }
    // Convert grapheme start → inclusive char-level end. For a 1-codepoint
    // grapheme this equals end_grapheme_start. For e + U+0301 (2 codepoints),
    // next_grapheme_boundary returns start+2, so end = start+1 (the combining
    // codepoint), ensuring the selection includes the full grapheme cluster.
    // Subtracting 1 is safe: the buffer always has a trailing '\n', so
    // next_grapheme_boundary is always > 0.
    let end = next_grapheme_boundary(buf, end_grapheme_start) - 1;

    Some((start, end))
}

/// Around word parameterised by boundary predicate.
///
/// Computes the inner word range, then extends to include surrounding
/// whitespace. The rule (matching Vim/Helix):
/// - If the word is a real word (non-whitespace): prefer trailing whitespace,
///   fall back to leading whitespace if no trailing whitespace exists.
/// - If the word IS whitespace: extend to include the adjacent non-whitespace
///   word that follows (or precedes if at end of line).
fn around_word_impl(
    buf: &Buffer,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> Option<(usize, usize)> {
    let (mut start, mut end) = inner_word_impl(buf, pos, is_boundary)?;
    let class = classify_char(buf.char_at(pos)?);

    // `next_pos` is the start of the grapheme cluster immediately after the
    // inner word. Since `end` is the last *char* of the last grapheme (as
    // returned by inner_word_impl), next_grapheme_boundary(end) gives the
    // first char of the following grapheme — equivalent to end + 1 for ASCII
    // but correct for multi-codepoint graphemes (e.g. e + combining accent).
    //
    // Similarly, `prev_start` is the start of the grapheme cluster immediately
    // before the inner word. Since `start` is always a grapheme-start position,
    // prev_grapheme_boundary(start) gives the start of the preceding grapheme —
    // equivalent to start - 1 for ASCII but safe for combining sequences.

    if class == CharClass::Space || class == CharClass::Eol {
        // The cursor is on whitespace. Extend to include the following word.
        // If there's no following word (e.g., at line end), include the
        // preceding word instead.
        let next_pos = next_grapheme_boundary(buf, end);
        if next_pos < buf.len_chars() {
            let next_class = classify_char(buf.char_at(next_pos)?);
            if next_class != CharClass::Space && next_class != CharClass::Eol {
                // Walk right to end of that word.
                let (_, word_end) = inner_word_impl(buf, next_pos, is_boundary)?;
                end = word_end;
            } else if start > 0 {
                // Try preceding word.
                let prev_start = prev_grapheme_boundary(buf, start);
                let prev_class = classify_char(buf.char_at(prev_start)?);
                if prev_class != CharClass::Space && prev_class != CharClass::Eol {
                    let (word_start, _) = inner_word_impl(buf, prev_start, is_boundary)?;
                    start = word_start;
                }
            }
        } else if start > 0 {
            let prev_start = prev_grapheme_boundary(buf, start);
            let prev_class = classify_char(buf.char_at(prev_start)?);
            if prev_class != CharClass::Space && prev_class != CharClass::Eol {
                let (word_start, _) = inner_word_impl(buf, prev_start, is_boundary)?;
                start = word_start;
            }
        }
    } else {
        // Real word. Try to include trailing whitespace first.
        let next_pos = next_grapheme_boundary(buf, end);
        if next_pos < buf.len_chars() {
            let next_class = classify_char(buf.char_at(next_pos)?);
            if next_class == CharClass::Space {
                // Walk right while whitespace.
                let (_, space_end) = inner_word_impl(buf, next_pos, is_word_boundary)?;
                end = space_end;
            } else {
                // No trailing space (next is Eol or another word) —
                // include leading whitespace instead.
                if start > 0 {
                    let prev_start = prev_grapheme_boundary(buf, start);
                    let prev_class = classify_char(buf.char_at(prev_start)?);
                    if prev_class == CharClass::Space {
                        let (space_start, _) = inner_word_impl(buf, prev_start, is_word_boundary)?;
                        start = space_start;
                    }
                }
            }
        } else if start > 0 {
            let prev_start = prev_grapheme_boundary(buf, start);
            let prev_class = classify_char(buf.char_at(prev_start)?);
            if prev_class == CharClass::Space {
                let (space_start, _) = inner_word_impl(buf, prev_start, is_word_boundary)?;
                start = space_start;
            }
        }
    }

    Some((start, end))
}

pub(crate) fn cmd_inner_word(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_text_object(&buf, sels, |b, pos| inner_word_impl(b, pos, is_word_boundary));
    (buf, new_sels)
}

pub(crate) fn cmd_around_word(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_text_object(&buf, sels, |b, pos| around_word_impl(b, pos, is_word_boundary));
    (buf, new_sels)
}

#[allow(non_snake_case)]
pub(crate) fn cmd_inner_WORD(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_text_object(&buf, sels, |b, pos| inner_word_impl(b, pos, is_WORD_boundary));
    (buf, new_sels)
}

#[allow(non_snake_case)]
pub(crate) fn cmd_around_WORD(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_text_object(&buf, sels, |b, pos| around_word_impl(b, pos, is_WORD_boundary));
    (buf, new_sels)
}

// ── Brackets ───────────────────────────────────────────────────────────────────

/// Scan left from `pos` (exclusive) to find an unmatched `open` bracket.
/// `depth` is the pre-loaded nesting depth (pass 0 when starting fresh).
fn scan_left_for_open(buf: &Buffer, pos: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = pos;
    loop {
        if i == 0 {
            return None;
        }
        i -= 1;
        match buf.char_at(i) {
            Some(ch) if ch == close => depth += 1,
            Some(ch) if ch == open => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
}

/// Scan right from `pos` (exclusive) to find an unmatched `close` bracket.
fn scan_right_for_close(buf: &Buffer, pos: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let len = buf.len_chars();
    let mut i = pos;
    while i < len {
        match buf.char_at(i) {
            Some(ch) if ch == open => depth += 1,
            Some(ch) if ch == close => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Find the innermost bracket pair `(open, close)` that encloses `pos`.
///
/// If the cursor is ON an open bracket, that bracket itself is the start.
/// If ON a close bracket, that bracket is the end.
/// Otherwise, scans both directions for the enclosing pair.
fn find_bracket_pair(buf: &Buffer, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    match buf.char_at(pos)? {
        ch if ch == open => {
            // Cursor is on an open bracket — scan right for the matching close.
            let close_pos = scan_right_for_close(buf, pos + 1, open, close)?;
            Some((pos, close_pos))
        }
        ch if ch == close => {
            // Cursor is on a close bracket — scan left for the matching open.
            let open_pos = scan_left_for_open(buf, pos, open, close)?;
            Some((open_pos, pos))
        }
        _ => {
            // Cursor is inside — scan both directions.
            let open_pos = scan_left_for_open(buf, pos, open, close)?;
            let close_pos = scan_right_for_close(buf, pos, open, close)?;
            Some((open_pos, close_pos))
        }
    }
}

fn inner_bracket(buf: &Buffer, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_bracket_pair(buf, pos, open, close)?;
    // Empty brackets: no valid inner range in the inclusive selection model.
    if open_pos + 1 > close_pos - 1 || close_pos == 0 {
        return None;
    }
    Some((open_pos + 1, close_pos - 1))
}

fn around_bracket(buf: &Buffer, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    find_bracket_pair(buf, pos, open, close)
}

macro_rules! bracket_cmds {
    ($inner_name:ident, $around_name:ident, $open:literal, $close:literal) => {
        pub(crate) fn $inner_name(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
            let new_sels =
                apply_text_object(&buf, sels, |b, pos| inner_bracket(b, pos, $open, $close));
            (buf, new_sels)
        }
        pub(crate) fn $around_name(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
            let new_sels =
                apply_text_object(&buf, sels, |b, pos| around_bracket(b, pos, $open, $close));
            (buf, new_sels)
        }
    };
}

bracket_cmds!(cmd_inner_paren, cmd_around_paren, '(', ')');
bracket_cmds!(cmd_inner_bracket, cmd_around_bracket, '[', ']');
bracket_cmds!(cmd_inner_brace, cmd_around_brace, '{', '}');
bracket_cmds!(cmd_inner_angle, cmd_around_angle, '<', '>');

// ── Quotes ─────────────────────────────────────────────────────────────────────

/// Find the quote pair on the current line that encloses or is nearest to `pos`.
///
/// Quotes don't span lines (M1 limitation). Strategy: scan the current line
/// tracking parity — odd occurrences are opening quotes, even occurrences are
/// closing quotes. Returns the pair that contains `pos`.
///
/// If `pos` is ON a quote char, parity resolves whether it is open or close.
fn find_quote_pair(buf: &Buffer, pos: usize, quote: char) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let line_start = buf.line_to_char(line);
    let line_end = line_end_exclusive(buf, line);

    // Single pass: track the opening quote position; on every second hit we
    // have a complete pair and can test whether `pos` falls inside it.
    let mut open: Option<usize> = None;
    for i in line_start..line_end {
        if buf.char_at(i) == Some(quote) {
            match open {
                None => open = Some(i), // odd occurrence → opening quote
                Some(open_pos) => {     // even occurrence → closing quote
                    if open_pos <= pos && pos <= i {
                        return Some((open_pos, i));
                    }
                    open = None; // reset for next pair
                }
            }
        }
    }
    None
}

fn inner_quote(buf: &Buffer, pos: usize, quote: char) -> Option<(usize, usize)> {
    let (open, close) = find_quote_pair(buf, pos, quote)?;
    // Empty quotes: no inner range.
    if open + 1 > close - 1 || close == 0 {
        return None;
    }
    Some((open + 1, close - 1))
}

fn around_quote(buf: &Buffer, pos: usize, quote: char) -> Option<(usize, usize)> {
    find_quote_pair(buf, pos, quote)
}

macro_rules! quote_cmds {
    ($inner_name:ident, $around_name:ident, $quote:literal) => {
        pub(crate) fn $inner_name(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
            let new_sels =
                apply_text_object(&buf, sels, |b, pos| inner_quote(b, pos, $quote));
            (buf, new_sels)
        }
        pub(crate) fn $around_name(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
            let new_sels =
                apply_text_object(&buf, sels, |b, pos| around_quote(b, pos, $quote));
            (buf, new_sels)
        }
    };
}

quote_cmds!(cmd_inner_double_quote, cmd_around_double_quote, '"');
quote_cmds!(cmd_inner_single_quote, cmd_around_single_quote, '\'');
quote_cmds!(cmd_inner_backtick, cmd_around_backtick, '`');

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_state;

    // ── Line ──────────────────────────────────────────────────────────────────

    #[test]
    fn inner_line_middle() {
        // Selection covers `world`, head=d (last char before \n).
        assert_state!(
            "hello\n-[w]>orld\nfoo\n",
            |(buf, sels)| cmd_inner_line(buf, sels),
            "hello\n-[world]>\nfoo\n"
        );
    }

    #[test]
    fn inner_line_start_of_line() {
        assert_state!(
            "-[h]>ello world\n",
            |(buf, sels)| cmd_inner_line(buf, sels),
            "-[hello world]>\n"
        );
    }

    #[test]
    fn inner_line_end_of_content() {
        assert_state!(
            "hello worl-[d]>\n",
            |(buf, sels)| cmd_inner_line(buf, sels),
            "-[hello world]>\n"
        );
    }

    #[test]
    fn inner_line_empty_line_is_noop() {
        // An empty line is just "\n" — no content, so inner_line returns None
        // and the selection is preserved.
        assert_state!(
            "hello\n-[\n]>world\n",
            |(buf, sels)| cmd_inner_line(buf, sels),
            "hello\n-[\n]>world\n"
        );
    }

    #[test]
    fn around_line_includes_newline() {
        // Selection covers `world\n`; head is the newline char.
        assert_state!(
            "hello\n-[w]>orld\nfoo\n",
            |(buf, sels)| cmd_around_line(buf, sels),
            "hello\n-[world\n]>foo\n"
        );
    }

    #[test]
    fn around_line_empty_line() {
        // An empty line is just "\n"; around_line selects that single char.
        // anchor == head, so serialises as a cursor (|).
        assert_state!(
            "hello\n-[\n]>world\n",
            |(buf, sels)| cmd_around_line(buf, sels),
            "hello\n-[\n]>world\n"
        );
    }

    // ── Word ──────────────────────────────────────────────────────────────────

    #[test]
    fn inner_word_middle() {
        // head=o (last char of `hello`).
        assert_state!(
            "-[h]>ello world\n",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "-[hello]> world\n"
        );
    }

    #[test]
    fn inner_word_cursor_at_end_of_word() {
        assert_state!(
            "hell-[o]> world\n",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "-[hello]> world\n"
        );
    }

    #[test]
    fn inner_word_cursor_on_whitespace() {
        // Two spaces between `foo` and `bar`; cursor on the first space.
        // inner_word selects the entire whitespace run (both spaces).
        // head = second space, serialised as `#[ | ]#`.
        assert_state!(
            "foo-[ ]> bar\n",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "foo-[  ]>bar\n"
        );
    }

    #[test]
    fn inner_word_cursor_on_punctuation() {
        // Both `!!` are Punctuation — selected as one run.
        assert_state!(
            "foo-[!]>!\n",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "foo-[!!]>\n"
        );
    }

    #[test]
    fn around_word_includes_trailing_space() {
        // Trailing space is included; head = the space char.
        assert_state!(
            "-[h]>ello world\n",
            |(buf, sels)| cmd_around_word(buf, sels),
            "-[hello ]>world\n"
        );
    }

    #[test]
    fn around_word_no_trailing_space_uses_leading() {
        // "world" at end of line has no trailing space, so leading space included.
        assert_state!(
            "hello -[w]>orld\n",
            |(buf, sels)| cmd_around_word(buf, sels),
            "hello-[ world]>\n"
        );
    }

    #[test]
    fn inner_word_includes_combining_grapheme() {
        // Buffer: "cafe\u{0301} world\n"
        // char offsets: c(0) a(1) f(2) e(3) ◌́(4) ' '(5) w(6) ...
        // Grapheme clusters: {c}{a}{f}{e◌́}{ }{w}...
        //
        // Old code (end += 1) stops at offset 3 because the combining codepoint
        // at offset 4 is classified as Punctuation — a false word/punct boundary
        // inside the grapheme. New code steps by grapheme boundary: the next
        // cluster after offset 3 starts at offset 5 (space), so the word ends
        // at offset 4 (last codepoint of the {e◌́} grapheme) — the full cluster
        // is included.
        assert_state!(
            "-[c]>afe\u{0301} world\n",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "-[cafe\u{0301}]> world\n"
        );
    }

    // ── WORD ──────────────────────────────────────────────────────────────────

    #[test]
    fn inner_WORD_spans_punctuation() {
        // `hello.world` is one WORD (no whitespace boundary within it).
        assert_state!(
            "-[h]>ello.world foo\n",
            |(buf, sels)| cmd_inner_WORD(buf, sels),
            "-[hello.world]> foo\n"
        );
    }

    // ── Brackets ──────────────────────────────────────────────────────────────

    #[test]
    fn inner_paren_cursor_inside() {
        assert_state!(
            "(-[h]>ello)\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "(-[hello]>)\n"
        );
    }

    #[test]
    fn around_paren_cursor_inside() {
        // around includes the parens themselves; head = `)`.
        assert_state!(
            "(-[h]>ello)\n",
            |(buf, sels)| cmd_around_paren(buf, sels),
            "-[(hello)]>\n"
        );
    }

    #[test]
    fn inner_paren_cursor_on_open() {
        // Cursor ON `(` — treated as if inside; same result as cursor inside.
        assert_state!(
            "-[(]>hello)\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "(-[hello]>)\n"
        );
    }

    #[test]
    fn inner_paren_cursor_on_close() {
        assert_state!(
            "(hello-[)]>\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "(-[hello]>)\n"
        );
    }

    #[test]
    fn inner_paren_empty_is_noop() {
        assert_state!(
            "-[(]>)\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "-[(]>)\n"
        );
    }

    #[test]
    fn inner_paren_nested_cursor_on_inner() {
        // Cursor inside inner `(b)` — selects `b`, which is a single char.
        // anchor == head, so serialises as a cursor.
        assert_state!(
            "(a(-[b]>)c)\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "(a(-[b]>)c)\n"
        );
    }

    #[test]
    fn inner_paren_nested_cursor_on_outer_content() {
        // Cursor on `a` (outside inner parens) — innermost enclosing pair
        // is the outer `(...)`, selects `a(b)c`.
        assert_state!(
            "(-[a]>(b)c)\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "(-[a(b)c]>)\n"
        );
    }

    #[test]
    fn inner_brace_basic() {
        assert_state!(
            "{-[h]>ello}\n",
            |(buf, sels)| cmd_inner_brace(buf, sels),
            "{-[hello]>}\n"
        );
    }

    #[test]
    fn inner_bracket_basic() {
        assert_state!(
            "[-[h]>ello]\n",
            |(buf, sels)| cmd_inner_bracket(buf, sels),
            "[-[hello]>]\n"
        );
    }

    #[test]
    fn inner_angle_basic() {
        assert_state!(
            "<-[h]>ello>\n",
            |(buf, sels)| cmd_inner_angle(buf, sels),
            "<-[hello]>>\n"
        );
    }

    #[test]
    fn inner_paren_no_match_is_noop() {
        assert_state!(
            "hel-[l]>o\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "hel-[l]>o\n"
        );
    }

    #[test]
    fn inner_paren_multiline() {
        // Bracket pair spans two lines; inner content is `\nhello\n`.
        // anchor = `\n` after `(`, head = `\n` before `)`.
        assert_state!(
            "(\n-[h]>ello\n)\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "(-[\nhello\n]>)\n"
        );
    }

    // ── Quotes ────────────────────────────────────────────────────────────────

    #[test]
    fn inner_double_quote_cursor_inside() {
        assert_state!(
            "\"hel-[l]>o\"\n",
            |(buf, sels)| cmd_inner_double_quote(buf, sels),
            "\"-[hello]>\"\n"
        );
    }

    #[test]
    fn around_double_quote_cursor_inside() {
        // around includes both quote chars; head = closing `"`.
        assert_state!(
            "\"hel-[l]>o\"\n",
            |(buf, sels)| cmd_around_double_quote(buf, sels),
            "-[\"hello\"]>\n"
        );
    }

    #[test]
    fn inner_double_quote_cursor_on_open() {
        assert_state!(
            "-[\"]>hello\"\n",
            |(buf, sels)| cmd_inner_double_quote(buf, sels),
            "\"-[hello]>\"\n"
        );
    }

    #[test]
    fn inner_double_quote_cursor_on_close() {
        assert_state!(
            "\"hello-[\"]>\n",
            |(buf, sels)| cmd_inner_double_quote(buf, sels),
            "\"-[hello]>\"\n"
        );
    }

    #[test]
    fn inner_double_quote_empty_is_noop() {
        assert_state!(
            "-[\"]>\"foo\n",
            |(buf, sels)| cmd_inner_double_quote(buf, sels),
            "-[\"]>\"foo\n"
        );
    }

    #[test]
    fn inner_double_quote_second_pair() {
        // Two pairs on the same line — cursor in second pair selects second.
        assert_state!(
            "\"a\" \"b-[c]>\"\n",
            |(buf, sels)| cmd_inner_double_quote(buf, sels),
            "\"a\" \"-[bc]>\"\n"
        );
    }

    #[test]
    fn inner_single_quote_basic() {
        assert_state!(
            "'hel-[l]>o'\n",
            |(buf, sels)| cmd_inner_single_quote(buf, sels),
            "'-[hello]>'\n"
        );
    }

    #[test]
    fn inner_backtick_basic() {
        assert_state!(
            "`hel-[l]>o`\n",
            |(buf, sels)| cmd_inner_backtick(buf, sels),
            "`-[hello]>`\n"
        );
    }

    #[test]
    fn inner_double_quote_not_inside_is_noop() {
        assert_state!(
            "hel-[l]>o\n",
            |(buf, sels)| cmd_inner_double_quote(buf, sels),
            "hel-[l]>o\n"
        );
    }

    // ── Multi-cursor ──────────────────────────────────────────────────────────

    #[test]
    fn inner_word_multi_cursor_different_words() {
        assert_state!(
            "-[h]>ello -[w]>orld\n",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "-[hello]> -[world]>\n"
        );
    }

    #[test]
    fn inner_word_multi_cursor_same_word_merges() {
        // Two cursors in the same word — both select "hello", merge to one selection.
        assert_state!(
            "-[h]>el-[l]>o world\n",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "-[hello]> world\n"
        );
    }

    #[test]
    fn around_word_multi_cursor() {
        // "hello world foo\n": cursor 0 on 'h'(0) → "hello "(0..5); cursor 1 on 'f'(12) → " foo"(11..14).
        assert_state!(
            "-[h]>ello world-[ ]>foo\n",
            |(buf, sels)| cmd_around_word(buf, sels),
            "-[hello ]>world-[ foo]>\n"
        );
    }

    #[test]
    fn inner_line_multi_cursor_same_line_merges() {
        // Two cursors on the same line both select that line's content, then merge.
        assert_state!(
            "-[h]>el-[l]>o\n",
            |(buf, sels)| cmd_inner_line(buf, sels),
            "-[hello]>\n"
        );
    }

    #[test]
    fn inner_line_multi_cursor_different_lines() {
        assert_state!(
            "-[h]>ello\n-[w]>orld\n",
            |(buf, sels)| cmd_inner_line(buf, sels),
            "-[hello]>\n-[world]>\n"
        );
    }

    #[test]
    fn around_line_multi_cursor_different_lines() {
        assert_state!(
            "-[h]>ello\n-[w]>orld\n",
            |(buf, sels)| cmd_around_line(buf, sels),
            "-[hello\n]>-[world\n]>"
        );
    }

    #[test]
    fn inner_WORD_multi_cursor() {
        assert_state!(
            "-[h]>ello.world -[f]>oo\n",
            |(buf, sels)| cmd_inner_WORD(buf, sels),
            "-[hello.world]> -[foo]>\n"
        );
    }

    #[test]
    fn inner_paren_two_cursors_same_pair_merge() {
        // Both cursors inside the same parens — both map to the same range → merge.
        assert_state!(
            "(-[h]>el-[l]>o)\n",
            |(buf, sels)| cmd_inner_paren(buf, sels),
            "(-[hello]>)\n"
        );
    }

    // ── around_WORD ───────────────────────────────────────────────────────────

    #[test]
    fn around_WORD_includes_trailing_space() {
        assert_state!(
            "-[h]>ello.world foo\n",
            |(buf, sels)| cmd_around_WORD(buf, sels),
            "-[hello.world ]>foo\n"
        );
    }

    #[test]
    fn around_WORD_no_trailing_space_uses_leading() {
        // Last WORD has no trailing space — grabs leading space instead.
        assert_state!(
            "hello.world -[f]>oo\n",
            |(buf, sels)| cmd_around_WORD(buf, sels),
            "hello.world-[ foo]>\n"
        );
    }

    #[test]
    fn around_WORD_cursor_on_whitespace_extends_to_next_WORD() {
        assert_state!(
            "foo-[ ]>bar\n",
            |(buf, sels)| cmd_around_WORD(buf, sels),
            "foo-[ bar]>\n"
        );
    }

    #[test]
    fn around_WORD_multi_cursor() {
        // "hello world foo\n": cursor on 'h'(0) → "hello "(0..5); cursor on 'f'(12) → " foo"(11..14).
        assert_state!(
            "-[h]>ello world-[ ]>foo\n",
            |(buf, sels)| cmd_around_WORD(buf, sels),
            "-[hello ]>world-[ foo]>\n"
        );
    }

    // ── around_bracket variants ───────────────────────────────────────────────

    #[test]
    fn around_brace_basic() {
        assert_state!(
            "{-[h]>ello}\n",
            |(buf, sels)| cmd_around_brace(buf, sels),
            "-[{hello}]>\n"
        );
    }

    #[test]
    fn around_bracket_basic() {
        assert_state!(
            "[-[h]>ello]\n",
            |(buf, sels)| cmd_around_bracket(buf, sels),
            "-[[hello]]>\n"
        );
    }

    #[test]
    fn around_angle_basic() {
        assert_state!(
            "<-[h]>ello>\n",
            |(buf, sels)| cmd_around_angle(buf, sels),
            "-[<hello>]>\n"
        );
    }

    // ── around_quote variants ─────────────────────────────────────────────────

    #[test]
    fn around_single_quote_basic() {
        assert_state!(
            "'hel-[l]>o'\n",
            |(buf, sels)| cmd_around_single_quote(buf, sels),
            "-['hello']>\n"
        );
    }

    #[test]
    fn around_backtick_basic() {
        assert_state!(
            "`hel-[l]>o`\n",
            |(buf, sels)| cmd_around_backtick(buf, sels),
            "-[`hello`]>\n"
        );
    }

    // ── multi-line bracket for non-paren types ────────────────────────────────

    #[test]
    fn inner_brace_multiline() {
        assert_state!(
            "{\n-[h]>ello\n}\n",
            |(buf, sels)| cmd_inner_brace(buf, sels),
            "{-[\nhello\n]>}\n"
        );
    }

    // ── edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn inner_word_on_structural_newline() {
        // Empty buffer: cursor on structural '\n'. inner_word selects the '\n'
        // (Eol class), which equals the original cursor — no visible change.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_inner_word(buf, sels),
            "-[\n]>"
        );
    }

    #[test]
    fn inner_WORD_on_structural_newline() {
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_inner_WORD(buf, sels),
            "-[\n]>"
        );
    }
}
