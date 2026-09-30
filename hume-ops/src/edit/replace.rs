//! Multi-cursor "replace around each head" primitives (`r`, and LSP
//! completion's fallback insert path) and word-boundary lookup.

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::grapheme::prev_grapheme_boundary;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::apply_edit;

/// Scans backward from `pos` over identifier (`Word`-class) chars, stopping
/// at the first non-`Word` boundary: the start of the token immediately
/// preceding `pos`. Grapheme-safe (steps via `prev_grapheme_boundary`, never
/// a raw `-= 1`). `chars` folds this buffer's extra word characters into the
/// scan, so a configured run (e.g. `foo-bar` with `-` as a word char) is
/// treated as one token, matching every other word operation, including
/// what the LSP completion fallback this backs is replacing on the buffer's
/// behalf.
pub fn word_start_before(text: &BufferText, pos: CharOffset, chars: WordChars<'_>) -> CharOffset {
    let mut cursor = pos;
    while cursor > CharOffset::new(0) {
        let prev = prev_grapheme_boundary(text, cursor);
        let Some(ch) = text.char_at(prev) else {
            break;
        };
        if chars.classify(ch) != CharClass::Word {
            break;
        }
        cursor = prev;
    }
    cursor
}

/// Multi-cursor "replace around each head": for every selection, delete from
/// `start_of(text, i, head)` through `forward` chars past the head and insert
/// `replacement`.
///
/// [`replace_around_cursors`] is the uniform case (a fixed backward char
/// count, valid for an LSP `textEdit` range since it contains the request
/// position). LSP completion's `insertText` fallback calls this directly,
/// because only the primary cursor's token start is tracked; other cursors
/// scan their own word start.
///
/// `text` may already include an earlier edit from the same group (e.g.
/// `additionalTextEdits`), so a fresh scan could wander into inserted text.
/// `i` (the cursor's sorted index) lets `start_of` read a per-cursor count
/// computed before that edit instead.
///
/// Where the delete ranges of two cursors overlap (cursors closer than the
/// span, or near the buffer start), the text they share is deleted once and
/// each cursor still receives `replacement`.
pub fn replace_span_around_cursors(
    state: EditState,
    start_of: impl Fn(&BufferText, usize, CharOffset) -> CharOffset,
    forward: usize,
    replacement: &str,
) -> Edited {
    apply_edit(state, |b, sel| {
        let text = b.text();
        let head = sel.head().offset();
        // The span comes from a char count taken at one cursor, so at another
        // it can cut a cluster: it widens to whole clusters.
        let chars = ExclusiveRange::new(
            start_of(text, sel.index(), head),
            head.shift(forward as isize),
        );
        let span = text
            .covering(chars)
            .map_or(ExclusiveRange::new(head, head), |range| range.chars());
        let mark = b.replace(span, replacement);
        Landing::after(mark.end())
    })
}

/// Replaces `back` chars behind each selection's head and `forward` chars
/// ahead of it with `replacement`: the multi-cursor form of "the user typed
/// this text here." Used by LSP completion accept for a server-provided
/// `textEdit` range: a conforming server's completion range always contains
/// the request position (LSP spec), so a `(back, forward)` pair derived from
/// one cursor's own edit is the same char span typing would have consumed at
/// any cursor, and applying it uniformly gives every cursor the completion,
/// not just the one the server saw.
pub fn replace_around_cursors(
    state: EditState,
    back: usize,
    forward: usize,
    replacement: &str,
) -> Edited {
    replace_span_around_cursors(
        state,
        // Saturating, not `retreat`: a cramped cursor's `head` can sit fewer
        // than `back` chars into the buffer.
        |_text, _i, head| head.retreat_saturating(back),
        forward,
        replacement,
    )
}

/// Replace every grapheme in every selection with `ch` (normal-mode `r`).
///
/// - **Cursor selection**: the single character under the cursor is replaced.
///   The cursor remains on the replacement character.
/// - **Multi-character selection**: every grapheme in the selected region is
///   replaced with `ch`, preserving the selection direction. Multi-codepoint
///   grapheme clusters (e.g. `é` = U+0065 + U+0301) are replaced atomically:
///   the replacement shrinks the cluster down to one char without orphaning
///   combining marks.
/// - **Newline skipping**: `\n` graphemes are never replaced; they are
///   retained as-is. This preserves line structure when the selection spans
///   multiple lines. The structural trailing `\n` is protected by the same
///   rule.
pub fn replace_selections(state: EditState, ch: char) -> Edited {
    apply_edit(state, |b, sel| {
        let text = b.text();
        // A cursor typing a pair char resolves open/close from what it sits
        // on; see `surround::smart_replace_char`.
        let current = crate::pair::delimiter_at(text, sel.start(), crate::surround::is_pair_char)
            .map(|(_, ch)| ch)
            .or_else(|| text.char_at(sel.start().offset()));
        let effective_ch = match current {
            Some(current) if sel.is_cursor() => {
                crate::surround::smart_replace_char(ch, current, sel.index())
            }
            _ => ch,
        };
        let replacement = effective_ch.to_string();
        let mark = b.keep(sel.covered());
        for cluster in sel.clusters() {
            if cluster.first() != '\n' {
                b.replace(cluster.range(), &replacement);
            }
        }
        Landing::covering(mark, sel.facing())
    })
}
