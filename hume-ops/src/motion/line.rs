use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::lines::{line_break_char, line_content_end, line_end_exclusive};
use hume_editing::text::BufferText;

// ── Line motions (inner) ──────────────────────────────────────────────────────

/// Jump to the first character on the current line.
pub(super) fn goto_line_start(text: &BufferText, head: usize) -> usize {
    text.line_to_char(text.char_to_line(head))
}

/// Jump to the last non-newline grapheme cluster on the current line.
///
/// On an empty line (containing only `\n`), the cursor stays on the newline —
/// there is no other character to land on.
pub(super) fn goto_line_end(text: &BufferText, head: usize) -> usize {
    // The core logic lives in hume_editing::lines::line_content_end, which is
    // also used by selection_cmd.rs — one implementation, two callers.
    line_content_end(text, text.char_to_line(head))
}

/// Jump to the `\n` that terminates the current line.
///
/// Unlike `goto_line_end` (which stops at the last non-newline grapheme and
/// therefore lands on the `\n` itself only on empty lines), this always
/// returns the `\n` position. Used by `cmd_open_line_below` to make the
/// insertion point uniform across empty and non-empty lines.
pub(super) fn goto_line_newline(text: &BufferText, head: usize) -> usize {
    let line = text.char_to_line(head);
    line_break_char(text, line)
}

/// Jump to the first non-blank character on the current line.
///
/// "Blank" means ASCII space or tab. If no non-blank character exists on the
/// line (e.g. a line of only spaces), the motion is a no-op and the cursor
/// stays at its current position.
pub(super) fn goto_first_nonblank(text: &BufferText, head: usize) -> usize {
    let line = text.char_to_line(head);
    let line_start = text.line_to_char(line);
    let end_excl = line_end_exclusive(text, line);

    let mut pos = line_start;
    while pos < end_excl {
        match text.char_at(pos) {
            // Step by grapheme boundary to respect the project invariant even
            // for space/tab (both are always single-codepoint, but be consistent).
            Some(' ') | Some('\t') => pos = next_grapheme_boundary(text, pos),
            Some('\n') | None => break, // end of line content without finding non-blank
            Some(_) => return pos,      // found a non-blank char
        }
    }
    head // no non-blank found — no-op, matching Helix
}
