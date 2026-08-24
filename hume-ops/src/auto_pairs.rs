use crate::edit::apply_edit;
use hume_editing::changeset::ChangeSet;
use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, classify_char};

// ── Config ────────────────────────────────────────────────────────────────────

/// A single bracket or quote pair for auto-pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub open: char,
    pub close: char,
}

impl Pair {
    /// True when the opening and closing characters are the same (e.g. `"` or `` ` ``).
    pub fn is_symmetric(&self) -> bool {
        self.open == self.close
    }
}

/// The auto-pair set: parentheses, brackets, braces, and the three quote
/// characters. Not runtime-configurable — no `:set` key or Steel setter
/// writes it; only `auto-pairs-enabled` (on/off) is a real setting.
pub const DEFAULT_PAIRS: &[Pair] = &[
    Pair {
        open: '(',
        close: ')',
    },
    Pair {
        open: '[',
        close: ']',
    },
    Pair {
        open: '{',
        close: '}',
    },
    Pair {
        open: '"',
        close: '"',
    },
    Pair {
        open: '\'',
        close: '\'',
    },
    Pair {
        open: '`',
        close: '`',
    },
];

// ── Edit functions ────────────────────────────────────────────────────────────

/// Insert an opening bracket and its matching close, placing the cursor
/// between them.
///
/// **Cursor selection** (anchor == head):
/// - Inserts `open` + `close` at the cursor position.
/// - Cursor lands on `close` so subsequent typed characters appear between
///   the pair (HUME's inclusive model: cursor sits on the character it will
///   displace, so typing pushes it right without an extra motion).
///
/// Multi-cursor: every selection is processed independently by `apply_edit`.
pub fn insert_pair_close(
    buf: BufferText,
    sels: SelectionSet,
    open: char,
    close: char,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, _buf, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start - b.old_pos());
        // Simple auto-close: insert open + close.
        b.insert_char(open);
        b.insert_char(close);
        // Cursor on `close`. new_pos - 1 is safe: we just inserted 2 chars.
        new_sels.push(Selection::collapsed(b.new_pos() - 1));
    })
}

/// Delete the bracket pair surrounding the cursor (the character before the
/// cursor and the character the cursor sits on), assuming the caller has
/// already verified that they form a configured pair.
///
/// Uses grapheme boundaries for correctness with multi-codepoint sequences,
/// even though bracket and quote characters are always single codepoints.
///
/// Only meaningful for cursor (single-character) selections; for non-cursor
/// selections the caller should fall back to `delete_char_backward`.
pub fn delete_pair(buf: BufferText, sels: SelectionSet) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        debug_assert!(
            sel.is_collapsed(),
            "delete_pair called on non-collapsed selection"
        );

        let p = sel.head();
        let prev = prev_grapheme_boundary(buf, p);
        let next = next_grapheme_boundary(buf, p);

        if prev < b.old_pos() {
            // A previous selection already consumed this region — treat as no-op.
            new_sels.push(Selection::collapsed(b.new_pos()));
            return;
        }

        // Delete from `prev` through `next` (exclusive), covering both the
        // char before the cursor and the char the cursor sits on.
        b.retain(prev - b.old_pos());
        b.delete(next - prev);
        new_sels.push(Selection::collapsed(b.new_pos()));
    })
}

// ── Context check ─────────────────────────────────────────────────────────────

/// Returns `true` if auto-pairing `pair` is appropriate when the cursor is at
/// `head` in `buf`.
///
/// Two conditions must hold:
/// 1. The character at `head` (what the cursor sits on) is "innocuous":
///    whitespace, newline, EOF, or a configured closing-pair character.
/// 2. For symmetric pairs (quotes/backticks): the character immediately before
///    `head` must NOT be a word character (alphanumeric or `_`). This prevents
///    auto-pairing inside words (e.g. typing `'` in `don't`) or after
///    identifier characters.
///
/// Callers are responsible for the all-or-nothing multi-cursor check; this
/// function evaluates a single cursor position.
pub fn should_auto_pair_at(buf: &BufferText, head: usize, pair: &Pair, ap_pairs: &[Pair]) -> bool {
    // Check 1: next char (the char the cursor sits on) must be innocuous.
    let next_ok = match buf.char_at(head) {
        None => true,                                     // EOF
        Some(c) if c.is_whitespace() => true,             // space, tab, newline, …
        Some(c) => ap_pairs.iter().any(|p| p.close == c), // a configured close char
    };
    if !next_ok {
        return false;
    }

    // Check 2 (symmetric pairs only): prev char must NOT be a word char.
    if pair.is_symmetric() && head > 0 {
        let prev_pos = prev_grapheme_boundary(buf, head);
        if buf
            .char_at(prev_pos)
            .is_some_and(|c| classify_char(c) == CharClass::Word)
        {
            return false;
        }
    }

    true
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
