use super::MotionMode;
use super::pair::{find_bracket_pair, find_quote_pair};
use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::lines::{is_empty_line, line_content_end, line_end_exclusive};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_editing::word::{CharClass, classify_char, is_uppercase_word_boundary, is_word_boundary};

// ── Text object framework ──────────────────────────────────────────────────────

/// Apply a text object to every selection in the set.
///
/// Unlike motions, which map a single cursor position to a new position, a
/// text object maps a cursor position to a *range* — the region to select.
/// `text_object` returns `Some((start, end))` as an inclusive char-offset pair,
/// or `None` if no match exists (e.g., cursor not inside any bracket pair).
///
/// On `None`, the existing selection is preserved — `mi(` when not inside parens
/// is a no-op. On `Some`, the selection is replaced with a
/// forward selection anchored at `start` and with head at `end`.
///
/// Uses `map` (which always merges) so that multiple cursors landing on the
/// same range (e.g., both cursors inside the same bracket pair) are merged.
pub(crate) fn apply_text_object(
    buf: &Text,
    sels: SelectionSet,
    text_object: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| match text_object(buf, sel.head()) {
        Some((start, end)) => Selection::new(start, end),
        None => sel,
    });
    result.debug_assert_valid(buf);
    result
}

/// Apply a text object in extend mode: union the matched range with the current selection.
///
/// On match, the result spans `min(sel.start(), start)` to `max(sel.end(), end)`,
/// preserving the direction of the original selection. On no-match, the selection
/// is unchanged.
///
/// Two-pass strategy for outward growth:
/// 1. Try `text_object(buf, sel.head)`. If the result is *larger* than the current
///    selection, use it — this handles the initial extend-from-cursor case.
/// 2. If the result is a subset (union doesn't grow), retry from the position just
///    past `sel.end()`. For bracket/quote text objects this escapes the current pair
///    and causes the search to find the next enclosing pair instead.
pub(crate) fn apply_text_object_extend(
    buf: &Text,
    sels: SelectionSet,
    text_object: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let forward = sel.anchor() <= sel.head();

        // First try from head (correct for initial extend from a cursor).
        if let Some((start, end)) = text_object(buf, sel.head()) {
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            if new_start != sel.start() || new_end != sel.end() {
                return Selection::directed(new_start, new_end, forward);
            }
        }

        // Result was a subset (no growth). Retry from one past the selection end so
        // bracket/quote searches find the enclosing pair rather than the current one.
        let past_end = next_grapheme_boundary(buf, sel.end());
        if past_end < buf.len_chars()
            && let Some((start, end)) = text_object(buf, past_end)
        {
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            return Selection::directed(new_start, new_end, forward);
        }

        sel
    });
    result.debug_assert_valid(buf);
    result
}

/// Dispatch to [`apply_text_object`] or [`apply_text_object_extend`] based on `mode`.
#[inline]
fn apply_text_object_by_mode(
    buf: &Text,
    sels: SelectionSet,
    mode: MotionMode,
    f: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_text_object(buf, sels, f),
        MotionMode::Extend => apply_text_object_extend(buf, sels, f),
    }
}

// ── Line ───────────────────────────────────────────────────────────────────────

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    if is_empty_line(buf, line) {
        return None; // empty line — no selectable content
    }
    let line_start = buf.line_to_char(line);
    // line_content_end returns the grapheme cluster *start* of the last
    // non-newline grapheme (uses prev_grapheme_boundary internally, so
    // combining clusters are handled correctly).
    let content_start = line_content_end(buf, line);
    // Convert grapheme start → last codepoint of that cluster, so the
    // selection includes all combining marks (same convention as inner_word).
    let end_inclusive = next_grapheme_boundary(buf, content_start).saturating_sub(1);
    Some((line_start, end_inclusive))
}

/// Around line: the full line including the trailing newline.
fn around_line(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    if end_excl == start {
        return None;
    }
    Some((start, end_excl - 1))
}

pub(crate) fn cmd_inner_line(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, inner_line)
}

pub(crate) fn cmd_around_line(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, around_line)
}

// ── Word / WORD ────────────────────────────────────────────────────────────────

/// Inner word parameterised by boundary predicate.
///
/// Scans left and right from `pos` while adjacent chars share the same
/// "class" (no boundary crossing). Whatever class the char at `pos` belongs
/// to defines the selected run — including whitespace runs and EOL.
pub(crate) fn inner_word_impl(
    buf: &Text,
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
    let end = next_grapheme_boundary(buf, end_grapheme_start) - 1; // grapheme-safe: result of next_grapheme_boundary is a cluster boundary; -1 is the last codepoint of the current cluster

    Some((start, end))
}

pub(crate) fn cmd_inner_word(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| {
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
pub(crate) fn expand_word_unit(buf: &Text, start: usize, end: usize) -> (usize, usize) {
    // Leading scan: walk back over Space graphemes from `start`. Stopping on
    // Eol means the run touches the start of the line — indentation.
    let mut run_start = start;
    let mut hit_eol = false;
    while run_start > 0 {
        let prev_pos = prev_grapheme_boundary(buf, run_start);
        match classify_char(buf.char_at(prev_pos).expect("prev_pos < len")) {
            CharClass::Space => run_start = prev_pos,
            CharClass::Eol => {
                hit_eol = true;
                break;
            }
            _ => break,
        }
    }
    let at_bol = hit_eol || run_start == 0;

    if run_start < start && !at_bol {
        return (run_start, end);
    }

    // Trailing fallback: first word of a line, punctuation immediately
    // before, or no adjacent whitespace at all.
    let mut run_end_start = end;
    loop {
        let next_pos = next_grapheme_boundary(buf, run_end_start);
        if next_pos >= buf.len_chars() {
            break;
        }
        if classify_char(buf.char_at(next_pos).expect("next_pos < len")) != CharClass::Space {
            break;
        }
        run_end_start = next_pos;
    }
    if run_end_start == end {
        (start, end)
    } else {
        // grapheme-safe: run_end_start is a grapheme boundary reached via
        // next_grapheme_boundary; -1 is the last codepoint of that cluster.
        (start, next_grapheme_boundary(buf, run_end_start) - 1)
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
pub(crate) fn word_unit_at(
    buf: &Text,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> Option<(usize, usize)> {
    // `pos` may be any valid selection endpoint, including the trailing
    // codepoint of a multi-codepoint grapheme cluster — see `anchor_unit`'s
    // doc for why this snap to the cluster start matters before classifying.
    let pos = prev_grapheme_boundary(buf, next_grapheme_boundary(buf, pos));
    let (start, end) = inner_word_impl(buf, pos, is_boundary)?;
    let class = classify_char(buf.char_at(pos)?);
    if class != CharClass::Space && class != CharClass::Eol {
        return Some(expand_word_unit(buf, start, end));
    }

    // On whitespace: `(start, end)` is the whitespace run — find the word
    // adjacent to it (following preferred, preceding fallback) and expand
    // that one by the normal rule instead.
    let is_word = |c: CharClass| c != CharClass::Space && c != CharClass::Eol;
    let next_pos = next_grapheme_boundary(buf, end);
    let word_pos = if next_pos < buf.len_chars() && is_word(classify_char(buf.char_at(next_pos)?)) {
        next_pos
    } else if start > 0 {
        let prev_pos = prev_grapheme_boundary(buf, start);
        if !is_word(classify_char(buf.char_at(prev_pos)?)) {
            return None;
        }
        prev_pos
    } else {
        return None;
    };
    let (start, end) = inner_word_impl(buf, word_pos, is_boundary)?;
    Some(expand_word_unit(buf, start, end))
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
pub(crate) fn nearest_word_on_line(
    buf: &Text,
    head: usize,
    line_start: usize,
    line_end_excl: usize,
    around: bool,
) -> Option<(usize, usize)> {
    let unit = |pos: usize| {
        if around {
            word_unit_at(buf, pos, is_word_boundary)
        } else {
            inner_word_impl(buf, pos, is_word_boundary)
        }
    };

    let class = classify_char(buf.char_at(head)?);

    // Fast path: head is already on a word/punct — delegate to inner/around unit.
    if class != CharClass::Space && class != CharClass::Eol {
        return unit(head);
    }

    // Scan LEFT within the given bounds for the first non-whitespace grapheme.
    let prev_anchor = {
        let mut pos = head;
        let mut found = None;
        while pos > line_start {
            pos = prev_grapheme_boundary(buf, pos);
            let c = classify_char(buf.char_at(pos)?);
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
            let next_pos = next_grapheme_boundary(buf, pos);
            if next_pos >= line_end_excl {
                break;
            }
            let c = classify_char(buf.char_at(next_pos)?);
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
/// preserving `sel.horiz` throughout.
///
/// Returns `sel` unchanged when `found` is `None` (no candidate word in bounds).
/// Shared by the buffer-line path in `cmd_select_word_nearest_on_line` and the
/// wrap-aware path in `cmd_visual_select_word_nearest_on_line`.
pub(crate) fn apply_nearest_word_result(
    sel: Selection,
    found: Option<(usize, usize)>,
    mode: MotionMode,
) -> Selection {
    let Some((start, end)) = found else {
        return sel;
    };
    match mode {
        MotionMode::Move => match sel.horiz() {
            Some(h) => Selection::with_horiz(start, end, h),
            None => Selection::new(start, end),
        },
        MotionMode::Extend => {
            let forward = sel.anchor() <= sel.head();
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            let s = Selection::directed(new_start, new_end, forward);
            match sel.horiz() {
                Some(h) => Selection::with_horiz(s.anchor(), s.head(), h),
                None => s,
            }
        }
    }
}

/// Select the word nearest the cursor on the same buffer line, snapping to it
/// when the cursor sits on whitespace. Preserves `sel.horiz` so the sticky
/// visual column (set by `move-down` / `move-up`) survives through this step.
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
pub(crate) fn cmd_select_word_nearest_on_line(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
    around: bool,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let line = buf.char_to_line(sel.anchor());
        let line_start = buf.line_to_char(line);
        let line_end_excl = line_end_exclusive(buf, line);
        let found = nearest_word_on_line(buf, sel.anchor(), line_start, line_end_excl, around);
        apply_nearest_word_result(sel, found, mode)
    });
    result.debug_assert_valid(buf);
    result
}

/// Around word (`ma w`): same span as `mm` (see [`cmd_select_word_around`]),
/// under a separate name because it stays available regardless of
/// `word-selects-whitespace`.
pub(crate) fn cmd_around_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    cmd_select_word_around(buf, sels, count, mode)
}

#[allow(non_snake_case)]
pub(crate) fn cmd_inner_uppercase_word(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| {
        inner_word_impl(b, pos, is_uppercase_word_boundary)
    })
}

/// Around WORD (`ma W`); see [`cmd_around_word`].
#[allow(non_snake_case)]
pub(crate) fn cmd_around_uppercase_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    cmd_select_uppercase_word_around(buf, sels, count, mode)
}

/// Select the word under the cursor (`mm`), covering its surrounding
/// whitespace per [`expand_word_unit`] — used when `word-selects-whitespace`
/// is on. Both modes use the same unit; `Extend` unions it with the current
/// selection via [`apply_text_object_extend`]. Also the body `maw` delegates
/// to — [`cmd_around_word`] — the two select the same span.
pub(crate) fn cmd_select_word_around(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| {
        word_unit_at(b, pos, is_word_boundary)
    })
}

/// Select the WORD under the cursor (`MM`); see [`cmd_select_word_around`].
#[allow(non_snake_case)]
pub(crate) fn cmd_select_uppercase_word_around(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| {
        word_unit_at(b, pos, is_uppercase_word_boundary)
    })
}

// ── Brackets ───────────────────────────────────────────────────────────────────

/// Shrink a `(open, close)` delimiter pair to its inner range, or `None` if
/// the pair is empty (no inner content in the inclusive selection model).
/// Shared by [`inner_bracket`] and [`inner_quote`].
fn inner_of_pair(open: usize, close: usize) -> Option<(usize, usize)> {
    if open + 1 > close - 1 || close == 0 {
        return None;
    }
    Some((open + 1, close - 1))
}

fn inner_bracket(buf: &Text, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_bracket_pair(buf, pos, open, close)?;
    inner_of_pair(open_pos, close_pos)
}

fn around_bracket(buf: &Text, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    find_bracket_pair(buf, pos, open, close)
}

macro_rules! bracket_cmds {
    ($inner_name:ident, $around_name:ident, $open:literal, $close:literal) => {
        pub(crate) fn $inner_name(
            buf: &Text,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| {
                inner_bracket(b, pos, $open, $close)
            })
        }
        pub(crate) fn $around_name(
            buf: &Text,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| {
                around_bracket(b, pos, $open, $close)
            })
        }
    };
}

bracket_cmds!(cmd_inner_paren, cmd_around_paren, '(', ')');
bracket_cmds!(cmd_inner_bracket, cmd_around_bracket, '[', ']');
bracket_cmds!(cmd_inner_brace, cmd_around_brace, '{', '}');
bracket_cmds!(cmd_inner_angle, cmd_around_angle, '<', '>');

// ── Quotes ─────────────────────────────────────────────────────────────────────

fn inner_quote(buf: &Text, pos: usize, quote: char) -> Option<(usize, usize)> {
    let (open, close) = find_quote_pair(buf, pos, quote)?;
    inner_of_pair(open, close)
}

fn around_quote(buf: &Text, pos: usize, quote: char) -> Option<(usize, usize)> {
    find_quote_pair(buf, pos, quote)
}

macro_rules! quote_cmds {
    ($inner_name:ident, $around_name:ident, $quote:literal) => {
        pub(crate) fn $inner_name(
            buf: &Text,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| inner_quote(b, pos, $quote))
        }
        pub(crate) fn $around_name(
            buf: &Text,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| around_quote(b, pos, $quote))
        }
    };
}

quote_cmds!(cmd_inner_double_quote, cmd_around_double_quote, '"');
quote_cmds!(cmd_inner_single_quote, cmd_around_single_quote, '\'');
quote_cmds!(cmd_inner_backtick, cmd_around_backtick, '`');

// ── Arguments (comma-separated items) ──────────────────────────────────────────

/// Find the tightest bracket pair among `()`, `[]`, `{}` that encloses `pos`.
///
/// Tries all three bracket types and returns the pair with the smallest span.
/// Tightest means innermost — for nested structures, we want the closest pair.
fn find_tightest_bracket_pair(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    PAIRS
        .iter()
        .filter_map(|&(open, close)| find_bracket_pair(buf, pos, open, close))
        .min_by_key(|&(o, c)| c - o)
}

/// Collect all comma-separated segments at depth 0 between `open_pos` and `close_pos`.
///
/// Returns a vec of `(start, end)` inclusive char-index pairs, one per segment,
/// including leading/trailing whitespace. Commas inside nested `()`, `[]`, or `{}`
/// are skipped. Returns an empty vec for adjacent brackets (`()`).
fn find_comma_segments(buf: &Text, open_pos: usize, close_pos: usize) -> Vec<(usize, usize)> {
    // Content zone: open_pos+1 ..= close_pos-1. Empty when brackets are adjacent.
    if close_pos <= open_pos + 1 {
        return Vec::new();
    }
    let content_start = open_pos + 1;
    let content_end = close_pos - 1; // inclusive

    let mut segments = Vec::new();
    let mut seg_start = content_start;
    let mut depth = 0usize;

    for (i, ch) in buf
        .chars_at(content_start)
        .take(content_end - content_start + 1)
    {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                // i - 1 >= seg_start - 1; safe since seg_start >= content_start >= 1.
                segments.push((seg_start, i - 1));
                seg_start = i + 1;
            }
            _ => {}
        }
    }

    // Final segment: everything after the last comma, or the whole content if no commas.
    segments.push((seg_start, content_end));
    segments
}

/// Find which segment in `segments` contains `pos`.
///
/// If `pos` falls in a gap (e.g., on a comma between two segments), associate
/// it with the following segment — matching Helix/Kakoune behaviour.
fn which_segment(segments: &[(usize, usize)], pos: usize) -> Option<usize> {
    // Direct containment.
    for (idx, &(start, end)) in segments.iter().enumerate() {
        if pos >= start && pos <= end {
            return Some(idx);
        }
    }
    // pos is in a gap (on a comma). Return the next segment.
    for idx in 0..segments.len().saturating_sub(1) {
        let (_, prev_end) = segments[idx];
        let (next_start, _) = segments[idx + 1];
        if pos > prev_end && pos < next_start {
            return Some(idx + 1);
        }
    }
    None
}

/// Inner argument: the text of the comma-separated item at `pos`, with leading
/// and trailing whitespace trimmed.
///
/// Works for function arguments `foo(a, b)`, array items `[1, 2]`, object
/// fields `{x: 1, y: 2}`, and any comma-separated list inside brackets.
fn inner_argument(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_tightest_bracket_pair(buf, pos)?;

    // Nudge: if the cursor is on a bracket itself, step into the content zone.
    let pos = if pos == open_pos {
        open_pos + 1
    } else if pos == close_pos {
        close_pos.saturating_sub(1)
    } else {
        pos
    };

    let segments = find_comma_segments(buf, open_pos, close_pos);
    if segments.is_empty() {
        return None;
    }

    let idx = which_segment(&segments, pos)?;
    let (raw_start, raw_end) = segments[idx];

    // Trim leading whitespace. next_grapheme_boundary is required here because
    // `start` is a text position — raw `+= 1` would mis-step on multi-byte clusters.
    let mut start = raw_start;
    while start <= raw_end && matches!(buf.char_at(start), Some(' ' | '\t' | '\n' | '\r')) {
        start = next_grapheme_boundary(buf, start);
    }
    // Trim trailing whitespace.
    let mut end = raw_end;
    while end > start && matches!(buf.char_at(end), Some(' ' | '\t' | '\n' | '\r')) {
        end = prev_grapheme_boundary(buf, end);
    }
    // Segment is entirely whitespace — nothing to select.
    if start > raw_end {
        return None;
    }

    Some((start, end))
}

/// Around argument: the item plus its separator comma, so that deleting around
/// leaves a clean, properly-spaced list.
///
/// - **Only arg**: same as inner (no separator to consume).
/// - **First arg**: extend end through the trailing comma and any whitespace
///   leading into the next argument, so `delete(around aaa)` in `foo(aaa, bbb)`
///   yields `foo(bbb)` with no leading space.
/// - **Non-first arg**: extend start back to include the preceding comma,
///   so `delete(around bbb)` in `foo(aaa, bbb)` yields `foo(aaa)`.
fn around_argument(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_tightest_bracket_pair(buf, pos)?;

    // Nudge cursor off the bracket itself.
    let pos = if pos == open_pos {
        open_pos + 1
    } else if pos == close_pos {
        close_pos.saturating_sub(1)
    } else {
        pos
    };

    let segments = find_comma_segments(buf, open_pos, close_pos);
    if segments.is_empty() {
        return None;
    }

    let idx = which_segment(&segments, pos)?;
    let (raw_start, raw_end) = segments[idx];

    if segments.len() == 1 {
        // Only argument — no separator to eat; same as inner.
        return inner_argument(buf, pos);
    }

    if idx == 0 {
        // First arg: eat the trailing comma and skip whitespace to the start
        // of the next argument's content, so no orphan space is left behind.
        let (next_raw_start, next_raw_end) = segments[1];
        let mut end = next_raw_start;
        while end <= next_raw_end && matches!(buf.char_at(end), Some(' ' | '\t')) {
            end = next_grapheme_boundary(buf, end);
        }
        // `end` is now the first content char of the next segment.
        // Our range is raw_start ..= (end - 1), eating "aaa, ".
        Some((raw_start, end - 1)) // grapheme-safe: end was advanced by next_grapheme_boundary; -1 is the last codepoint of the preceding (whitespace) cluster
    } else {
        // Non-first arg: eat the preceding comma (it sits at prev_raw_end + 1).
        // The raw segment already includes any leading space after the comma,
        // so this range covers ", bbb" naturally.
        let prev_raw_end = segments[idx - 1].1;
        Some((prev_raw_end + 1, raw_end)) // grapheme-safe: comma is single-codepoint ASCII
    }
}

pub(crate) fn cmd_inner_argument(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, inner_argument)
}

pub(crate) fn cmd_around_argument(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, around_argument)
}

#[cfg(test)]
mod tests;
