use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::lines::{is_empty_line, line_content_end};
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;

use super::{MotionMode, apply_object_motion};

// ── Paragraph span (shared with `text_object::paragraph`) ──────────────────────

/// The last char a selection may cover on `line`: the last codepoint of its
/// final grapheme cluster (so a trailing combining mark is never orphaned),
/// or the line's own `\n` when the line is empty.
///
/// `line_content_end` answers with that cluster's *start* — where a cursor
/// lands — so the round trip through `next_grapheme_boundary` converts to
/// its last codepoint; an identity on the single-codepoint clusters most
/// text is made of, the `\n` of an empty line included.
fn paragraph_last_char(text: &BufferText, line: usize) -> usize {
    next_grapheme_boundary(text, line_content_end(text, line)).saturating_sub(1)
}

/// First line of the paragraph containing the non-empty line `line`.
///
/// One walk shared by both callers: [`current_paragraph_start`] resolves a
/// cursor to its own paragraph, while `prev_paragraph_start`'s backward scan
/// lands on the *last* line of its target and has to climb from there.
fn paragraph_first_line(text: &BufferText, line: usize) -> usize {
    let mut first = line;
    while first > 0 && !is_empty_line(text, first - 1) {
        first -= 1;
    }
    first
}

/// Inclusive char span of the paragraph starting at `first_line`, optionally
/// extended through its trailing blank gap.
///
/// Content domain throughout (`content_line_count`, never the ropey line
/// count). The phantom trailing line reads as empty too — its own start and
/// terminator both sit at `len_chars()` — so a gap scan in the ropey domain
/// would swallow it and hand back a span end of `len_chars()`, one past what
/// the `head < len_chars()` cursor invariant allows. The position-returning
/// motion this replaced could afford the ropey count because it never used
/// the landing line as a span end, only as a cursor position.
///
/// A paragraph with no trailing gap is exactly one that ends the buffer, so
/// `paragraph_last_char` on the unextended end line already answers both
/// cases — no separate EOF branch needed.
pub(crate) fn paragraph_span(
    text: &BufferText,
    first_line: usize,
    include_gap: bool,
) -> (usize, usize) {
    let total = text.content_line_count();

    let mut end_line = first_line;
    while end_line + 1 < total && !is_empty_line(text, end_line + 1) {
        end_line += 1;
    }
    if include_gap {
        while end_line + 1 < total && is_empty_line(text, end_line + 1) {
            end_line += 1;
        }
    }

    (
        text.line_to_char(first_line),
        paragraph_last_char(text, end_line),
    )
}

/// First line of the paragraph enclosing `pos`, or `None` on a blank line —
/// there is no paragraph there to select.
pub(crate) fn current_paragraph_start(text: &BufferText, pos: usize) -> Option<usize> {
    let line = text.char_to_line(pos);
    (!is_empty_line(text, line)).then(|| paragraph_first_line(text, line))
}

// ── Paragraph motions (`}` / `{`) ───────────────────────────────────────────────

/// First line of the paragraph strictly after `pos`'s own paragraph and its
/// gap, or `None` at EOF.
fn next_paragraph_start(text: &BufferText, pos: usize) -> Option<usize> {
    let total = text.content_line_count();
    let mut line = text.char_to_line(pos);

    // Skip the current paragraph (non-empty lines), then its gap (empty
    // lines) — landing past `total` means there's nothing below.
    while line < total && !is_empty_line(text, line) {
        line += 1;
    }
    while line < total && is_empty_line(text, line) {
        line += 1;
    }

    (line < total).then_some(line)
}

/// First line of the paragraph strictly before `pos`'s own paragraph, or
/// `None` if none exists (blank lines, or the enclosing paragraph itself,
/// reach the buffer start with no gap in between).
///
/// Starting inside a blank-line gap is handled separately from starting
/// inside paragraph text: the nearest paragraph above the gap is the
/// target, not skipped past — mirroring how `next_paragraph_start` treats a
/// gap as belonging to no paragraph rather than eating an extra one.
fn prev_paragraph_start(text: &BufferText, pos: usize) -> Option<usize> {
    let mut line = text.char_to_line(pos);

    if is_empty_line(text, line) {
        while line > 0 && is_empty_line(text, line) {
            line -= 1;
        }
        if is_empty_line(text, line) {
            return None; // blank all the way to BOF — nothing above
        }
    } else {
        while line > 0 && !is_empty_line(text, line) {
            line -= 1;
        }
        if line == 0 {
            return None; // this paragraph starts at line 0 — nothing above it
        }
        while line > 0 && is_empty_line(text, line) {
            line -= 1;
        }
        if is_empty_line(text, line) {
            return None; // blank all the way to BOF — nothing above
        }
    }

    // `line` is the last line of the previous paragraph — climb to its first.
    Some(paragraph_first_line(text, line))
}

/// Span of the next paragraph, plus its trailing gap.
fn find_next_paragraph(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    Some(paragraph_span(text, next_paragraph_start(text, pos)?, true))
}

/// Span of the previous paragraph, plus its trailing gap.
fn find_prev_paragraph(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    Some(paragraph_span(text, prev_paragraph_start(text, pos)?, true))
}

/// Select the next paragraph, plus its trailing blank gap (`}`). No-op if
/// there is no paragraph below.
pub fn cmd_goto_next_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_object_motion(text, sels, mode, count, false, |pos| {
        find_next_paragraph(text, pos)
    })
}

/// Select the previous paragraph, plus its trailing blank gap (`{`). No-op
/// if there is no paragraph above.
pub fn cmd_goto_prev_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_object_motion(text, sels, mode, count, true, |pos| {
        find_prev_paragraph(text, pos)
    })
}
