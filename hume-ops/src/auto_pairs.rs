use crate::edit::apply_edit;
use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars};
use hume_rope::cluster::{ClusterBound, ClusterRange};

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
/// characters. Not runtime-configurable: no `:set` key or Steel setter
/// writes it; only `auto-pairs` (on/off) is a real setting.
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
pub fn insert_pair_close(state: EditState, open: char, close: char) -> Edited {
    apply_edit(state, |b, sel| {
        b.insert(sel.start(), open.encode_utf8(&mut [0; 4]));
        let closer = b.insert(sel.start(), close.encode_utf8(&mut [0; 4]));
        Landing::cursor(closer.start())
    })
}

/// Delete the bracket pair surrounding the cursor (the character before the
/// cursor and the character the cursor sits on), assuming the caller has
/// already verified that they form a configured pair.
///
/// Only meaningful for cursors; for selections the caller falls back to
/// `delete_char_backward`.
pub fn delete_pair(state: EditState) -> Edited {
    apply_edit(state, |b, sel| {
        debug_assert!(sel.is_cursor(), "delete_pair called on a selection");
        let head = sel.head();
        let text = b.text();
        let next = text.clusters().end_of(head);
        let Some(pair) = text
            .clusters()
            .prev(head.into())
            .and_then(|prev| ClusterRange::between(text.full_slice(), prev, next))
        else {
            return Landing::kept(sel.selection());
        };
        Landing::cursor(b.delete(pair))
    })
}

// ── Context check ─────────────────────────────────────────────────────────────

/// Returns `true` if auto-pairing `pair` is appropriate when the cursor is at
/// `head` in `text`.
///
/// Two conditions must hold:
/// 1. The character at `head` (what the cursor sits on) is "innocuous":
///    whitespace, newline, EOF, or a configured closing-pair character.
/// 2. For symmetric pairs (quotes/backticks): the character immediately before
///    `head` must be neither a word character (this buffer's configured
///    `word-chars` folded in via `chars`, same as every other word
///    operation) nor the pair's own character. This prevents auto-pairing
///    inside words (e.g. typing `'` in `don't`), after identifier
///    characters, and within a run of the same quote (a Markdown fence or
///    a `"""` string).
///
/// Callers are responsible for the all-or-nothing multi-cursor check; this
/// function evaluates a single cursor position.
pub fn should_auto_pair_at(
    text: &BufferText,
    head: ClusterBound,
    pair: &Pair,
    ap_pairs: &[Pair],
    chars: WordChars<'_>,
) -> bool {
    // Check 1: next char (the char the cursor sits on) must be innocuous.
    let next_ok = match text.char_at(head.offset()) {
        None => true,                                     // EOF
        Some(c) if c.is_whitespace() => true,             // space, tab, newline, …
        Some(c) => ap_pairs.iter().any(|p| p.close == c), // a configured close char
    };
    if !next_ok {
        return false;
    }

    // Check 2 (symmetric pairs only): prev char must be neither a word char
    // nor the pair's own char.
    if pair.is_symmetric()
        && let Some(prev) = text.clusters().prev(head)
        && text
            .char_at(prev.offset())
            .is_some_and(|c| c == pair.open || chars.classify(c) == CharClass::Word)
    {
        return false;
    }

    true
}

#[cfg(test)]
mod tests;
