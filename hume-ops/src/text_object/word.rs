//! Word/WORD text objects (`iw`/`aw`, `iW`/`aW`) and the position-based
//! `mm`/`MM`/nearest-word-on-line family they share with visual-move.

use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::lines::line_end_exclusive;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, classify_char, is_uppercase_word_boundary, is_word_boundary};

use super::apply_text_object_by_mode;
use crate::MotionMode;

/// Inner word parameterised by boundary predicate.
///
/// Scans left and right from `pos` while adjacent chars share the same
/// "class" (no boundary crossing). Whatever class the char at `pos` belongs
/// to defines the selected run — including whitespace runs and EOL.
pub fn inner_word_impl(
    text: &BufferText,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> Option<(usize, usize)> {
    let class = classify_char(text.char_at(pos)?);

    // Scan left: walk back by grapheme cluster boundaries while the preceding
    // grapheme belongs to the same class. Using prev_grapheme_boundary ensures
    // we always inspect the *base* codepoint of each grapheme (not a combining
    // codepoint like U+0301 that would be misclassified as Punctuation).
    let mut start = pos;
    while start > 0 {
        let prev_pos = prev_grapheme_boundary(text, start);
        let prev = classify_char(text.char_at(prev_pos)?);
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
        let next_pos = next_grapheme_boundary(text, end_grapheme_start);
        if next_pos >= text.len_chars() {
            break;
        }
        let next = classify_char(text.char_at(next_pos)?);
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
    let end = next_grapheme_boundary(text, end_grapheme_start) - 1; // grapheme-safe: result of next_grapheme_boundary is a cluster boundary; -1 is the last codepoint of the current cluster

    Some((start, end))
}

pub fn cmd_inner_word(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, |b, pos| {
        inner_word_impl(b, pos, is_word_boundary)
    })
}

/// Grow a word/punct span `(start, end)` to include an adjacent whitespace
/// run: leading preferred, trailing when the leading run is indentation or
/// absent.
///
/// A leading run that reaches back to the start of its line (or the start of
/// the buffer) is indentation, not inter-word spacing, and must never be
/// absorbed — the first word of a line always takes its trailing whitespace
/// instead. This keeps `w`/`b`/`mm`/`maw` from ever eating indentation.
///
/// `min_start` is a hard lower bound on the leading scan, never crossed.
/// Buffer-line callers pass `0` (no floor beyond the buffer itself). The wrap
/// path passes the visual sub-row's start so a word beginning a continuation
/// row never absorbs the inter-word space that lives at the end of the
/// previous display row.
///
/// Reaching `min_start` only counts as indentation (blocking absorption) when
/// `min_start` is itself a genuine line start — the buffer start, or right
/// after a real newline. A wrap sub-row boundary is neither: it falls
/// mid-line, so a leading run that reaches it is ordinary inter-word spacing
/// that happens to sit at the row split, not indentation, and stays
/// absorbable up to that floor.
pub fn expand_word_unit(
    text: &BufferText,
    start: usize,
    end: usize,
    min_start: usize,
) -> (usize, usize) {
    let min_start_is_bol = min_start == 0
        || classify_char(
            text.char_at(prev_grapheme_boundary(text, min_start))
                .expect("min_start > 0 implies a preceding char"),
        ) == CharClass::Eol;

    // Leading scan: walk back over Space graphemes from `start`. Stopping on
    // Eol means the run touches the start of the line — indentation.
    let mut run_start = start;
    let mut hit_eol = false;
    while run_start > min_start {
        let prev_pos = prev_grapheme_boundary(text, run_start);
        match classify_char(text.char_at(prev_pos).expect("prev_pos < len")) {
            CharClass::Space => run_start = prev_pos,
            CharClass::Eol => {
                hit_eol = true;
                break;
            }
            _ => break,
        }
    }
    let at_bol = hit_eol || (run_start == min_start && min_start_is_bol);

    if run_start < start && !at_bol {
        return (run_start, end);
    }

    // Trailing fallback: first word of a line, punctuation immediately
    // before, or no adjacent whitespace at all.
    let mut run_end_start = end;
    loop {
        let next_pos = next_grapheme_boundary(text, run_end_start);
        if next_pos >= text.len_chars() {
            break;
        }
        if classify_char(text.char_at(next_pos).expect("next_pos < len")) != CharClass::Space {
            break;
        }
        run_end_start = next_pos;
    }
    if run_end_start == end {
        (start, end)
    } else {
        // grapheme-safe: run_end_start is a grapheme boundary reached via
        // next_grapheme_boundary; -1 is the last codepoint of that cluster.
        (start, next_grapheme_boundary(text, run_end_start) - 1)
    }
}

/// The word (or WORD) unit at `pos`: the inner word plus its whitespace
/// bookend per [`expand_word_unit`].
///
/// When `pos` sits on whitespace there is no word under the cursor — snap to
/// the adjacent word (the one right after the run if any, else the one right
/// before it) and expand that instead. The whitespace under the cursor is
/// never selected for its own sake; it only appears in the span when the
/// expansion re-absorbs it (an inter-word space run is the following word's
/// leading run), so newlines and indentation never leak into the selection.
/// Returns `None` when no word is adjacent to the run (e.g. a
/// whitespace-only buffer, or indentation at the start of the buffer) — the
/// callers treat that as a no-op.
///
/// This is the shared body of `mm`/`MM` and `maw`/`maW` (position-based,
/// unlike the motion-based `w`/`b`) — all four names select the same span.
/// Also used to resolve an extend selection's anchor unit when
/// `word-selects-whitespace` is on.
pub fn word_unit_at(
    text: &BufferText,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    min_start: usize,
) -> Option<(usize, usize)> {
    // `pos` may be any valid selection endpoint, including the trailing
    // codepoint of a multi-codepoint grapheme cluster — see `anchor_unit`'s
    // doc for why this snap to the cluster start matters before classifying.
    let pos = prev_grapheme_boundary(text, next_grapheme_boundary(text, pos));
    let (start, end) = inner_word_impl(text, pos, is_boundary)?;
    let class = classify_char(text.char_at(pos)?);
    if class != CharClass::Space && class != CharClass::Eol {
        return Some(expand_word_unit(text, start, end, min_start));
    }

    // On whitespace: `(start, end)` is the whitespace run — find the word
    // adjacent to it (following preferred, preceding fallback) and expand
    // that one by the normal rule instead.
    let is_word = |c: CharClass| c != CharClass::Space && c != CharClass::Eol;
    let next_pos = next_grapheme_boundary(text, end);
    let word_pos = if next_pos < text.len_chars() && is_word(classify_char(text.char_at(next_pos)?))
    {
        next_pos
    } else if start > 0 {
        let prev_pos = prev_grapheme_boundary(text, start);
        if !is_word(classify_char(text.char_at(prev_pos)?)) {
            return None;
        }
        prev_pos
    } else {
        return None;
    };
    let (start, end) = inner_word_impl(text, word_pos, is_boundary)?;
    Some(expand_word_unit(text, start, end, min_start))
}

/// Find the nearest word within `[line_start, line_end_excl)` from `head`.
///
/// - If `head` is on a word or punctuation char, returns its inner-word range
///   (identical to `inner_word_impl`), or the word plus its whitespace
///   bookend (identical to `word_unit_at`) when `around` is set.
/// - If `head` is on whitespace or EOL, scans left and right within the given
///   bounds to find the closest word. "Closest" is measured as the distance
///   from `head` to the nearest edge of each candidate word; ties go to the
///   previous (left) word. The winning word is then resolved the same way
///   (inner vs. around) as the direct-hit case.
/// - Returns `None` when no word exists within the bounds.
///
/// Callers supply bounds explicitly so this helper can be scoped to either a
/// buffer line (no-wrap path) or a visual sub-row (wrap path). `around`
/// mirrors the effective `word-selects-whitespace` setting — see
/// `cmd_select_word_nearest_on_line` and `cmd_visual_select_word_nearest_on_line`.
pub fn nearest_word_on_line(
    text: &BufferText,
    head: usize,
    line_start: usize,
    line_end_excl: usize,
    around: bool,
) -> Option<(usize, usize)> {
    let unit = |pos: usize| {
        if around {
            word_unit_at(text, pos, is_word_boundary, line_start)
        } else {
            inner_word_impl(text, pos, is_word_boundary)
        }
    };

    let class = classify_char(text.char_at(head)?);

    // Fast path: head is already on a word/punct — delegate to inner/around unit.
    if class != CharClass::Space && class != CharClass::Eol {
        return unit(head);
    }

    // Scan LEFT within the given bounds for the first non-whitespace grapheme.
    let prev_anchor = {
        let mut pos = head;
        let mut found = None;
        while pos > line_start {
            pos = prev_grapheme_boundary(text, pos);
            let c = classify_char(text.char_at(pos)?);
            if c != CharClass::Space && c != CharClass::Eol {
                found = Some(pos);
                break;
            }
        }
        found
    };

    // Scan RIGHT within the given bounds for the first non-whitespace grapheme.
    let next_anchor = {
        let mut pos = head;
        let mut found = None;
        loop {
            let next_pos = next_grapheme_boundary(text, pos);
            if next_pos >= line_end_excl {
                break;
            }
            let c = classify_char(text.char_at(next_pos)?);
            if c != CharClass::Space && c != CharClass::Eol {
                found = Some(next_pos);
                break;
            }
            pos = next_pos;
        }
        found
    };

    match (prev_anchor, next_anchor) {
        (None, None) => None,
        (Some(p), None) => unit(p),
        (None, Some(n)) => unit(n),
        (Some(p), Some(n)) => {
            // Pick the word whose nearest edge is closer to `head`; tie → prev.
            // `p` is the last char of the prev word's run (nearest edge = p itself).
            // `n` is the first char of the next word's run (nearest edge = n itself).
            let dist_prev = head.saturating_sub(p);
            let dist_next = n.saturating_sub(head);
            let anchor = if dist_next < dist_prev { n } else { p };
            unit(anchor)
        }
    }
}

/// Apply the result of `nearest_word_on_line` to `sel` according to `mode`,
/// preserving `sel.sticky_display_col` throughout.
///
/// Returns `sel` unchanged when `found` is `None` (no candidate word in bounds).
/// Shared by the buffer-line path in `cmd_select_word_nearest_on_line` and the
/// wrap-aware path in `cmd_visual_select_word_nearest_on_line`.
pub fn apply_nearest_word_result(
    sel: Selection,
    found: Option<(usize, usize)>,
    mode: MotionMode,
) -> Selection {
    let Some((start, end)) = found else {
        return sel;
    };
    match mode {
        MotionMode::Move => match sel.sticky_display_col() {
            Some(sticky) => Selection::with_sticky_display_col(start, end, sticky),
            None => Selection::new(start, end),
        },
        MotionMode::Extend => {
            let forward = sel.anchor() <= sel.head();
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            let s = Selection::directed(new_start, new_end, forward);
            match sel.sticky_display_col() {
                Some(sticky) => Selection::with_sticky_display_col(s.anchor(), s.head(), sticky),
                None => s,
            }
        }
    }
}

/// Select the word nearest the cursor on the same buffer line, snapping to it
/// when the cursor sits on whitespace. Preserves `sel.sticky_display_col` so
/// the sticky display column (set by `move-down` / `move-up`) survives
/// through this step.
///
/// `around` mirrors the effective `word-selects-whitespace` setting: when set,
/// the selected span includes the word's whitespace bookend (matching `mm`);
/// when unset, only the inner word is selected.
///
/// In wrap mode, `cmd_visual_select_word_nearest_on_line` (in `editor/visual_move.rs`)
/// should be used instead — it scopes the search to the current visual sub-row,
/// preventing the snap from reaching across a wrap boundary.
///
/// In `Extend` mode the matched word range is unioned with the existing
/// selection, matching the behaviour of `inner-word` in extend mode.
pub fn cmd_select_word_nearest_on_line(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
    around: bool,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let line = text.char_to_line(sel.anchor());
        let line_start = text.line_to_char(line);
        let line_end_excl = line_end_exclusive(text, line);
        let found = nearest_word_on_line(text, sel.anchor(), line_start, line_end_excl, around);
        apply_nearest_word_result(sel, found, mode)
    });
    result.debug_assert_valid(text);
    result
}

/// Around word (`ma w`): same span as `mm` (see [`cmd_select_word_around`]),
/// under a separate name because it stays available regardless of
/// `word-selects-whitespace`.
pub fn cmd_around_word(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    cmd_select_word_around(text, sels, count, mode)
}

#[allow(non_snake_case)]
pub fn cmd_inner_uppercase_word(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, |b, pos| {
        inner_word_impl(b, pos, is_uppercase_word_boundary)
    })
}

/// Around WORD (`ma W`); see [`cmd_around_word`].
#[allow(non_snake_case)]
pub fn cmd_around_uppercase_word(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    cmd_select_uppercase_word_around(text, sels, count, mode)
}

/// Select the word under the cursor (`mm`), covering its surrounding
/// whitespace per [`expand_word_unit`] — used when `word-selects-whitespace`
/// is on. Both modes use the same unit; `Extend` unions it with the current
/// selection via `apply_text_object_extend`. Also the body `maw` delegates
/// to — [`cmd_around_word`] — the two select the same span.
pub fn cmd_select_word_around(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, |b, pos| {
        word_unit_at(b, pos, is_word_boundary, 0)
    })
}

/// Select the WORD under the cursor (`MM`); see [`cmd_select_word_around`].
#[allow(non_snake_case)]
pub fn cmd_select_uppercase_word_around(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, |b, pos| {
        word_unit_at(b, pos, is_uppercase_word_boundary, 0)
    })
}
