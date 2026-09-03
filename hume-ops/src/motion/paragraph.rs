use hume_editing::lines::{is_empty_line, line_last_char};
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;

use super::{MotionMode, apply_object_motion};

// ── Paragraph span (shared with `text_object::paragraph`) ──────────────────────

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
/// `line_last_char` on the unextended end line already answers both cases —
/// no separate EOF branch needed.
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
        line_last_char(text, end_line),
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
/// Mirrors `next_paragraph_start`'s two-phase walk in the other direction:
/// leave the enclosing paragraph, then its gap. Starting inside a blank-line
/// gap skips the first phase on its own (its condition is already false) and
/// lands on the nearest paragraph above rather than skipping past it — the
/// same "gap belongs to no paragraph" rule `next_paragraph_start` applies
/// forward. Running out of buffer during either phase means there is nothing
/// above.
fn prev_paragraph_start(text: &BufferText, pos: usize) -> Option<usize> {
    let mut line = text.char_to_line(pos);

    while !is_empty_line(text, line) {
        line = line.checked_sub(1)?;
    }
    while is_empty_line(text, line) {
        line = line.checked_sub(1)?;
    }

    // `line` is the last line of the previous paragraph — climb to its first.
    Some(paragraph_first_line(text, line))
}

/// Select the next paragraph, plus its trailing blank gap (`}`). No-op if
/// there is no paragraph below.
///
/// Not a `motion_cmd!`: the finder yields a span, not a head — the same
/// `apply_object_motion` the structural `goto-next-<kind>` family uses.
pub fn cmd_goto_next_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_object_motion(text, sels, mode, count, false, |pos| {
        Some(paragraph_span(text, next_paragraph_start(text, pos)?, true))
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
        Some(paragraph_span(text, prev_paragraph_start(text, pos)?, true))
    })
}
