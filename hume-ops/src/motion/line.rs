use hume_editing::grapheme::graphemes_at;
use hume_editing::lines::{line_break_char, line_content_end, next_line_start};
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;

// ── Line motions (inner) ──────────────────────────────────────────────────────

/// Jump to the first character on the current line.
pub(super) fn goto_line_start(text: &BufferText, head: CharOffset) -> CharOffset {
    text.line_to_char(text.char_to_line(head).into())
}

/// Jump to the last non-newline grapheme cluster on the current line.
///
/// On an empty line (containing only `\n`), the cursor stays on the newline:
/// there is no other character to land on.
pub(super) fn goto_line_end(text: &BufferText, head: CharOffset) -> CharOffset {
    // The core logic lives in hume_editing::lines::line_content_end, which is
    // also used by selection_cmd.rs: one implementation, two callers.
    line_content_end(text, text.char_to_line(head))
}

/// Jump to the `\n` that terminates the current line.
///
/// Unlike `goto_line_end` (which stops at the last non-newline grapheme and
/// therefore lands on the `\n` itself only on empty lines), this always
/// returns the `\n` position.
pub(super) fn goto_line_newline(text: &BufferText, head: CharOffset) -> CharOffset {
    let line = text.char_to_line(head);
    line_break_char(text, line)
}

/// Jump to the first non-blank character on the current line.
///
/// "Blank" means ASCII space or tab. If no non-blank character exists on the
/// line (e.g. a line of only spaces), the motion is a no-op and the cursor
/// stays at its current position.
pub(super) fn goto_first_nonblank(text: &BufferText, head: CharOffset) -> CharOffset {
    let line = text.char_to_line(head);
    let line_start = text.line_to_char(line.into());
    let end_excl = next_line_start(text, line.into());

    for cluster in graphemes_at(text, line_start).take_while(|cluster| cluster.start < end_excl) {
        match cluster.first {
            ' ' | '\t' => {}
            '\n' => break,             // end of line content without finding non-blank
            _ => return cluster.start, // found a non-blank char
        }
    }
    head // no non-blank found: no-op, matching Helix
}
