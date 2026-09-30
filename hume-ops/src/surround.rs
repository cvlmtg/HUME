//! Surround operations: select the delimiter characters of an enclosing pair.
//!
//! `ms` + char selects the surrounding delimiters as two cursor selections,
//! enabling standard select-then-act composition:
//! - `ms(` → `d`  deletes the parens
//! - `ms(` → `r[` replaces `()` with `[]` (via smart replace)
//! - `ms(` → `c`  enters insert with two cursors on the delimiters
//!
//! Deliberately not Helix's `md`/`mr`, which bake the selection and the
//! action together as a single keystroke. That violates select-then-act.

use crate::MotionMode;
use crate::edit::apply_edit;
use crate::pair::{find_bracket_pair, find_quote_pair};
use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::selection::Selection;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_rope::cluster::{ClusterRange, ClusterStart};

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

/// Wrap the content of every selection (including single-char cursors) with
/// `open` + selected_text + `close`. The content stops before the `\n` a
/// selection ends on, and a selection covering only a `\n` has nothing to
/// wrap.
///
/// Cursor placement: lands on the `close` character after the wrapped content.
/// Multi-cursor: each selection is wrapped independently via `apply_edit`.
pub fn wrap_each_selection(state: EditState, open: char, close: char) -> Edited {
    apply_edit(state, |b, sel| {
        let Some(content) = sel.content() else {
            return Landing::kept(sel.selection());
        };
        b.insert(content.start(), open.encode_utf8(&mut [0; 4]));
        let closer = b.insert(content.end(), close.encode_utf8(&mut [0; 4]));
        Landing::cursor(closer.start())
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

/// Shared implementation: map each selection to two cursors on the pair's
/// delimiter clusters, or preserve unchanged on no-match.
fn select_surround(
    state: EditState,
    find_pair: impl Fn(&BufferText, ClusterStart) -> Option<ClusterRange>,
) -> EditState {
    state.flat_map(|sel| {
        let (first, second) = match find_pair(sel.text(), sel.head()) {
            Some(pair) => (
                Selection::cursor(pair.start()),
                Some(Selection::cursor(pair.last())),
            ),
            None => (sel.selection(), None),
        };
        std::iter::once(first).chain(second)
    })
}

// ── Generated surround commands ──────────────────────────────────────────────

macro_rules! surround_cmd {
    ($name:ident, bracket, $open:literal, $close:literal) => {
        pub fn $name(state: EditState, _count: usize, _mode: MotionMode) -> EditState {
            select_surround(state, |t, pos| find_bracket_pair(t, pos, $open, $close))
        }
    };
    ($name:ident, quote, $quote:literal) => {
        pub fn $name(state: EditState, _count: usize, _mode: MotionMode) -> EditState {
            select_surround(state, |t, pos| find_quote_pair(t, pos, $quote))
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

#[cfg(test)]
mod tests;
