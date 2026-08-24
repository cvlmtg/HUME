//! Surround operations: select the delimiter characters of an enclosing pair.
//!
//! `ms` + char selects the surrounding delimiters as two cursor selections,
//! enabling standard select-then-act composition:
//! - `ms(` → `d`  deletes the parens
//! - `ms(` → `r[` replaces `()` with `[]` (via smart replace)
//! - `ms(` → `c`  enters insert with two cursors on the delimiters
//!
//! Deliberately not Helix's `md`/`mr`, which bake the selection and the
//! action together as a single keystroke — that violates select-then-act.

use crate::MotionMode;
use crate::edit::apply_edit;
use crate::pair::{find_bracket_pair, find_quote_pair};
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;

// ── Pair lookup ──────────────────────────────────────────────────────────────

/// All recognised delimiter pairs.  Asymmetric first, then symmetric.
///
/// Intentionally a superset of the default auto-pair set: angle brackets
/// (`<>`) are useful for surround-select in markup, but shouldn't auto-close
/// in insert mode where `<` is commonly a comparison operator.
const PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

fn pair_for_char(ch: char) -> Option<(char, char)> {
    PAIRS.iter().find(|&&(o, c)| o == ch || c == ch).copied()
}

fn is_opening(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o != c && o == ch)
}

fn is_closing(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o != c && c == ch)
}

fn is_symmetric(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o == c && o == ch)
}

// ── Wrap selections ──────────────────────────────────────────────────────────

/// Wrap every selection — including single-char cursors — with `open` + selected_text + `close`.
///
/// Cursor placement: lands on the `close` character after the wrapped content.
/// Multi-cursor: each selection is wrapped independently via `apply_edit`.
pub fn wrap_each_selection(
    text: BufferText,
    sels: SelectionSet,
    open: char,
    close: char,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        let start = sel.start();
        let end_incl = sel
            .end_inclusive(text)
            .min(text.len_chars().saturating_sub(2));
        // When `start` sits on (or past) the structural trailing '\n', there's nothing
        // user-visible to wrap. Skip so `insert_char(close)` is never placed after '\n'.
        if start >= text.len_chars() - 1 {
            new_sels.push(Selection::collapsed(b.new_pos()));
            return;
        }
        b.retain(start - b.old_pos());
        b.insert_char(open);
        b.retain(end_incl + 1 - start); // copy selected text through — no String alloc
        b.insert_char(close);
        // Cursor on the close char. new_pos - 1 is safe: close is always preceded by
        // at least open + one retained char (HUME selections are ≥ 1 char).
        new_sels.push(Selection::collapsed(b.new_pos() - 1));
    })
}

// ── Smart replace resolution ─────────────────────────────────────────────────

/// Resolve the effective replacement character for pair-aware replace.
///
/// When the user types `r[` and the cursor sits on `(`, this returns `[`.
/// When the cursor sits on `)`, this returns `]`.  For symmetric source
/// chars (quotes) the selection index breaks the tie: even = opening,
/// odd = closing.
///
/// Returns `replacement` unchanged when:
/// - `replacement` is not part of any known pair, or
/// - `current` is not a known delimiter character.
pub(crate) fn smart_replace_char(replacement: char, current: char, sel_index: usize) -> char {
    let (open, close) = match pair_for_char(replacement) {
        Some(p) => p,
        None => return replacement,
    };

    if is_opening(current) {
        open
    } else if is_closing(current) {
        close
    } else if is_symmetric(current) {
        // Symmetric source (e.g. `"` → `(`): use selection index as
        // tiebreaker.  After `ms"` the first cursor (even index) sits on
        // the opening quote, the second (odd) on the closing quote.
        if sel_index.is_multiple_of(2) {
            open
        } else {
            close
        }
    } else {
        replacement
    }
}

// ── Select surrounding delimiters ────────────────────────────────────────────

/// Shared implementation: map each selection to two cursors on the pair
/// endpoints, or preserve unchanged on no-match.
fn select_surround(
    text: &BufferText,
    sels: SelectionSet,
    find_pair: impl Fn(&BufferText, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let primary_idx = sels.primary_index();
    let mut new_sels = Vec::with_capacity(sels.len() * 2);
    let mut new_primary = 0;

    for (i, sel) in sels.iter_sorted().enumerate() {
        if i == primary_idx {
            new_primary = new_sels.len();
        }
        if let Some((open_pos, close_pos)) = find_pair(text, sel.head()) {
            new_sels.push(Selection::collapsed(open_pos));
            new_sels.push(Selection::collapsed(close_pos));
        } else {
            new_sels.push(*sel);
        }
    }

    let result = SelectionSet::from_vec(new_sels, new_primary);
    result.debug_assert_valid(text);
    result
}

// ── Generated surround commands ──────────────────────────────────────────────

macro_rules! surround_cmd {
    ($name:ident, bracket, $open:literal, $close:literal) => {
        pub fn $name(
            text: &BufferText,
            sels: SelectionSet,
            _count: usize,
            _mode: MotionMode,
        ) -> SelectionSet {
            select_surround(text, sels, |b, pos| find_bracket_pair(b, pos, $open, $close))
        }
    };
    ($name:ident, quote, $quote:literal) => {
        pub fn $name(
            text: &BufferText,
            sels: SelectionSet,
            _count: usize,
            _mode: MotionMode,
        ) -> SelectionSet {
            select_surround(text, sels, |b, pos| find_quote_pair(b, pos, $quote))
        }
    };
}

surround_cmd!(cmd_surround_paren, bracket, '(', ')');
surround_cmd!(cmd_surround_bracket, bracket, '[', ']');
surround_cmd!(cmd_surround_brace, bracket, '{', '}');
surround_cmd!(cmd_surround_angle, bracket, '<', '>');
surround_cmd!(cmd_surround_double_quote, quote, '"');
surround_cmd!(cmd_surround_single_quote, quote, '\'');
surround_cmd!(cmd_surround_backtick, quote, '`');

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
