use std::iter::Peekable;

use hume_editing::lines::line_last_char;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_rope::lines::is_empty_line_token;
use ropey::RopeSlice;

use super::{MotionMode, apply_object_motion};

// ── Line-run scanning ────────────────────────────────────────────────────────
//
// A paragraph is a run of content lines, optionally followed by a run of
// blank lines (its gap). Every finder below is a sequence of such runs off
// one cursor per direction — `next_if` (not `take_while`) is what makes that
// composable: it leaves the run-ending token unconsumed, so a second run can
// continue the same cursor exactly where the first stopped, rather than
// forcing a fresh one to re-seek to that boundary.

/// Count of consecutive tokens at the front of `tokens` whose emptiness
/// matches `empty`. The token that ends the run stays unconsumed.
fn run<'a>(tokens: &mut Peekable<impl Iterator<Item = RopeSlice<'a>>>, empty: bool) -> usize {
    std::iter::from_fn(|| tokens.next_if(|t| is_empty_line_token(*t) == empty)).count()
}

/// Count of consecutive non-empty tokens at the front of `tokens`.
fn content_run<'a>(tokens: &mut Peekable<impl Iterator<Item = RopeSlice<'a>>>) -> usize {
    run(tokens, false)
}

/// Count of consecutive empty tokens at the front of `tokens`.
fn blank_run<'a>(tokens: &mut Peekable<impl Iterator<Item = RopeSlice<'a>>>) -> usize {
    run(tokens, true)
}

/// Tokens from `line` forward, stopping before the phantom trailing line —
/// the content-domain bound every forward scan below needs. Its token is
/// empty, so left unbounded it would read as part of a trailing gap.
fn content_tokens_at(
    text: &BufferText,
    line: usize,
) -> Peekable<impl Iterator<Item = RopeSlice<'_>>> {
    text.line_tokens_at(line)
        .take(text.content_line_count() - line)
        .peekable()
}

/// Inclusive char span from `first_line`'s start to `last_line`'s last
/// content char.
fn line_span(text: &BufferText, first_line: usize, last_line: usize) -> (usize, usize) {
    (
        text.line_to_char(first_line),
        line_last_char(text, last_line),
    )
}

// ── Paragraph finders ────────────────────────────────────────────────────────
//
// Each finder touches every line it scans exactly once: a run measured on
// one cursor is continued on that same cursor rather than re-opened at a
// boundary an earlier phase already found.

/// Paragraph enclosing `pos`, as an inclusive char span — its own lines, plus
/// the trailing blank gap when `include_gap`. `None` on a blank line: there
/// is no paragraph there to select.
pub(crate) fn paragraph_at(
    text: &BufferText,
    pos: usize,
    include_gap: bool,
) -> Option<(usize, usize)> {
    let line = text.char_to_line(pos);

    // Climb backward from `line` itself (a run of 0 means `line` is blank —
    // the "no paragraph here" case) to the paragraph's first line.
    let mut back = text.line_tokens_back_from(line).peekable();
    let up = content_run(&mut back);
    if up == 0 {
        return None;
    }
    let first = line + 1 - up;

    // Continue forward from `line`'s own successor — `first..=line` is
    // already known to be content, so there's nothing left to scan there.
    let mut fwd = content_tokens_at(text, line + 1).peekable();
    let mut last = line + content_run(&mut fwd);
    if include_gap {
        last += blank_run(&mut fwd);
    }

    Some(line_span(text, first, last))
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
        next_paragraph(text, pos)
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
        prev_paragraph(text, pos)
    })
}

/// The paragraph strictly after `pos`'s own paragraph, plus its trailing gap,
/// or `None` at EOF. One forward cursor: leave the enclosing paragraph, then
/// its gap (landing past the buffer's last content line means there's
/// nothing below), then the target paragraph's own content and gap.
fn next_paragraph(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let total = text.content_line_count();
    let line = text.char_to_line(pos);
    let mut tokens = content_tokens_at(text, line).peekable();

    let mut target = line + content_run(&mut tokens);
    target += blank_run(&mut tokens);
    if target >= total {
        return None;
    }

    // Same cursor, already positioned at `target`.
    let content_end = target + content_run(&mut tokens) - 1;
    let last = content_end + blank_run(&mut tokens);
    Some(line_span(text, target, last))
}

/// The paragraph strictly before `pos`'s own paragraph, plus its trailing
/// gap, or `None` if none exists.
///
/// One backward cursor for three phases — leave the enclosing paragraph,
/// then its gap (running out of buffer during either means there's nothing
/// above), then climb the target paragraph to its own first line — plus one
/// forward cursor to measure the target's trailing gap, which the backward
/// walk can't answer: starting inside a gap that continues below `pos`, the
/// backward count only sees the blanks at or above `pos`, never the ones
/// below it.
fn prev_paragraph(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let line = text.char_to_line(pos);
    let mut back = text.line_tokens_back_from(line).peekable();

    let after_paragraph = line.checked_sub(content_run(&mut back))?;
    let target_last = after_paragraph.checked_sub(blank_run(&mut back))?;

    // Same cursor, already positioned at `target_last`.
    let up = content_run(&mut back);
    let first = target_last + 1 - up;

    let mut fwd = content_tokens_at(text, target_last + 1).peekable();
    let last = target_last + blank_run(&mut fwd);

    Some(line_span(text, first, last))
}
