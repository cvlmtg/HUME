//! Multi-cursor "replace around each head" primitives (`r`, and LSP
//! completion's fallback insert path) and word-boundary lookup.

use hume_editing::changeset::ChangeSet;
use hume_editing::grapheme::{
    next_grapheme_boundary, prev_grapheme_boundary, snap_to_cluster_start,
};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars};

use super::apply_edit;

/// Scans backward from `pos` over identifier (`Word`-class) chars, stopping
/// at the first non-`Word` boundary — the start of the token immediately
/// preceding `pos`. Grapheme-safe (steps via `prev_grapheme_boundary`, never
/// a raw `-= 1`). `chars` folds this buffer's extra word characters into the
/// scan, so a configured run (e.g. `foo-bar` with `-` as a word char) is
/// treated as one token — matching every other word operation, including
/// what the LSP completion fallback this backs is replacing on the buffer's
/// behalf.
pub fn word_start_before(text: &BufferText, pos: usize, chars: WordChars<'_>) -> usize {
    let mut cursor = pos;
    while cursor > 0 {
        let prev = prev_grapheme_boundary(text, cursor);
        let Some(ch) = text.char_at(prev) else { break };
        if chars.classify(ch) != CharClass::Word {
            break;
        }
        cursor = prev;
    }
    cursor
}

/// General multi-cursor "replace around each head" primitive: for every
/// selection, `start_of(text, head)` determines where the deletion begins and
/// `forward` chars ahead of the head are replaced along with it, uniformly,
/// by `replacement`. [`replace_around_cursors`] is the common case (`start_of`
/// is a uniform backward char count). LSP completion's `insertText` fallback
/// (no server-provided range) calls this directly instead, since it has no
/// single uniform notion of "how far back" — its own `start_of` closure
/// special-cases the session's primary cursor (whose true token start is
/// `CompletionSession::anchor()`, tracked independently of live buffer
/// content — see that method's doc for why) and falls back to a per-cursor
/// scan for every other cursor.
///
/// Two cursors closer together than the resulting span, or a cursor nearer
/// the buffer start than it, would otherwise produce a delete range starting
/// before `b.old_pos()` (the previous selection's edit already claimed that
/// text) — clamped to `b.old_pos()` instead of erroring, so a cramped cursor
/// simply replaces less and every cursor still receives `replacement`.
pub fn replace_span_around_cursors(
    text: BufferText,
    sels: SelectionSet,
    start_of: impl Fn(&BufferText, usize) -> usize,
    forward: usize,
    replacement: &str,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        let head = sel.head();
        // `start_of(head)`/`head + forward` bound a char span that can land
        // mid-cluster when this cursor's surrounding text differs from the
        // one the span was derived from (e.g. a combining mark). Snap
        // outward — floor `start` down, ceil `end` up — to the enclosing
        // cluster boundary rather than splitting it.
        let raw_start = start_of(text, head);
        let start = snap_to_cluster_start(text, raw_start).max(b.old_pos());
        // Capped at `len_chars()` (not `len_chars() - 1`) so the boundary
        // lookups below never see an out-of-range offset; the structural
        // newline itself is protected by the overshoot check just after.
        let raw_end = (head + forward).min(text.len_chars()).max(start);
        let ceiled = if raw_end == 0 {
            0
        } else {
            // Ceil is `start`'s mirror: `prev` then `next` is identity when
            // `raw_end` is already a boundary, otherwise the start of the
            // next cluster.
            next_grapheme_boundary(text, prev_grapheme_boundary(text, raw_end))
        };
        // `len_chars() - 1` is the buffer's structural trailing `\n` — never
        // consume it. The cap above must not run *after* the ceil: a
        // mid-cluster `raw_end` capped early and then ceiled (e.g. the final
        // cluster is `\r\n` — a lone `\r` survives normalization and gets a
        // structural `\n` appended after it) would ceil right back past the
        // newline. Floor back to that cluster's own start instead of
        // splitting it.
        let last = text.len_chars() - 1;
        let end = if ceiled > last {
            prev_grapheme_boundary(text, ceiled).max(start)
        } else {
            ceiled
        };
        b.retain(start - b.old_pos());
        b.delete(end - start);
        b.insert(replacement);
        let sel = Selection::collapsed(b.new_pos());
        new_sels.push(sel);
    })
}

/// Replaces `back` chars behind each selection's head and `forward` chars
/// ahead of it with `replacement` — the multi-cursor form of "the user typed
/// this text here." Used by LSP completion accept for a server-provided
/// `textEdit` range: a conforming server's completion range always contains
/// the request position (LSP spec), so a `(back, forward)` pair derived from
/// one cursor's own edit is the same char span typing would have consumed at
/// any cursor, and applying it uniformly gives every cursor the completion,
/// not just the one the server saw.
pub fn replace_around_cursors(
    text: BufferText,
    sels: SelectionSet,
    back: usize,
    forward: usize,
    replacement: &str,
) -> (BufferText, SelectionSet, ChangeSet) {
    replace_span_around_cursors(
        text,
        sels,
        |_buf, head| head.saturating_sub(back),
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
///   grapheme clusters (e.g. `é` = U+0065 + U+0301) are replaced atomically —
///   the replacement shrinks the cluster down to one char without orphaning
///   combining marks.
/// - **Newline skipping**: `\n` graphemes are never replaced — they are
///   retained as-is. This preserves line structure when the selection spans
///   multiple lines. The structural trailing `\n` is protected by the same
///   rule.
pub fn replace_selections(
    text: BufferText,
    sels: SelectionSet,
    ch: char,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, i, sel, new_sels| {
        let sel_start = sel.start();
        let sel_end = sel.end(); // inclusive last-grapheme-start; equal to sel_start for cursor

        // Smart replace: when replacing a single character (cursor selection)
        // and the replacement is a pair character, resolve open/close based on
        // what's currently under the cursor.  See `surround::smart_replace_char`.
        let effective_ch = if sel.is_collapsed() {
            if let Some(current) = text.char_at(sel_start) {
                crate::surround::smart_replace_char(ch, current, i)
            } else {
                ch
            }
        } else {
            ch
        };

        // Retain everything up to this selection (handles the gap from the
        // previous selection or the buffer start). Record the start position
        // in result-buffer coordinates for later selection reconstruction.
        b.retain(sel_start - b.old_pos());
        let new_sel_start = b.new_pos();

        let mut pos = sel_start;
        loop {
            let next = next_grapheme_boundary(text, pos);
            // `\n` graphemes are skipped (retained) to preserve line structure.
            // This also naturally protects the structural trailing '\n'.
            if text.char_at(pos) == Some('\n') {
                b.retain(next - pos);
            } else {
                // After the initial `retain` above, b.old_pos() == sel_start == pos.
                // Each subsequent delete advances b.old_pos() by the cluster size,
                // landing exactly at the next grapheme start — so the builder stays
                // in sync without additional retain calls between graphemes.
                b.delete(next - pos);
                b.insert_char(effective_ch);
            }
            if pos >= sel_end {
                break;
            }
            pos = next;
        }
        // new_pos() is one past the last written char — the final grapheme of the
        // replaced range. -1 gives the cursor position (inclusive last char).
        let new_sel_end = b.new_pos() - 1;

        // Reconstruct the selection with its original direction.
        // `Selection::directed` is the canonical constructor for this pattern:
        // it takes content-aware (start, end) bounds and a direction flag.
        let forward = sel.anchor() <= sel.head();
        new_sels.push(Selection::directed(new_sel_start, new_sel_end, forward));
    })
}
