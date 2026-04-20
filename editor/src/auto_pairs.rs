use crate::core::text::Text;
use crate::core::changeset::ChangeSet;
use crate::ops::edit::apply_edit;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::core::selection::{Selection, SelectionSet};
use crate::helpers::{classify_char, CharClass};

// ── Config ────────────────────────────────────────────────────────────────────

/// A single bracket or quote pair for auto-pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pair {
    pub open: char,
    pub close: char,
}

impl Pair {
    /// True when the opening and closing characters are the same (e.g. `"` or `` ` ``).
    pub(crate) fn is_symmetric(&self) -> bool {
        self.open == self.close
    }
}

// ── Edit functions ────────────────────────────────────────────────────────────

/// Insert an opening bracket and its matching close, placing the cursor
/// between them. If the selection is non-empty, wrap the selected text with
/// the pair instead.
///
/// **Cursor selection** (anchor == head):
/// - Inserts `open` + `close` at the cursor position.
/// - Cursor lands on `close` so subsequent typed characters appear between
///   the pair (HUME's inclusive model: cursor sits on the character it will
///   displace, so typing pushes it right without an extra motion).
///
/// **Non-cursor selection**:
/// - Wraps the selected text: `open` + selected_text + `close`.
/// - Cursor lands on `close`.
///
/// Multi-cursor: every selection is processed independently by `apply_edit`.
pub(crate) fn insert_pair_close(
    buf: Text,
    sels: SelectionSet,
    open: char,
    close: char,
) -> (Text, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start - b.old_pos());

        if sel.is_collapsed() {
            // Simple auto-close: insert open + close.
            b.insert_char(open);
            b.insert_char(close);
            // Cursor on `close`. new_pos - 1 is safe: we just inserted 2 chars.
            new_sels.push(Selection::collapsed(b.new_pos() - 1));
        } else {
            // Wrap selection: read the selected text, delete it, re-insert
            // with open/close around it.
            let end_incl = sel.end_inclusive(buf).min(buf.len_chars().saturating_sub(2));
            let selected: String = buf.slice(start..end_incl + 1).to_string();
            b.delete(end_incl + 1 - start);
            b.insert_char(open);
            b.insert(&selected);
            b.insert_char(close);
            // Cursor on the close bracket. new_pos - 1 is safe: we just inserted open + selected + close (≥ 2 chars).
            new_sels.push(Selection::collapsed(b.new_pos() - 1));
        }
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
pub(crate) fn delete_pair(
    buf: Text,
    sels: SelectionSet,
) -> (Text, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        debug_assert!(sel.is_collapsed(), "delete_pair called on non-collapsed selection");

        let p = sel.head;
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
pub(crate) fn should_auto_pair_at(buf: &Text, head: usize, pair: &Pair, ap_pairs: &[Pair]) -> bool {
    // Check 1: next char (the char the cursor sits on) must be innocuous.
    let next_ok = match buf.char_at(head) {
        None => true,                         // EOF
        Some(c) if c.is_whitespace() => true, // space, tab, newline, …
        Some(c) => ap_pairs.iter().any(|p| p.close == c), // a configured close char
    };
    if !next_ok {
        return false;
    }

    // Check 2 (symmetric pairs only): prev char must NOT be a word char.
    if pair.is_symmetric() && head > 0 {
        let prev_pos = prev_grapheme_boundary(buf, head);
        if let Some(prev_ch) = buf.char_at(prev_pos) {
            if classify_char(prev_ch) == CharClass::Word {
                return false;
            }
        }
    }

    true
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_state;

    // ── insert_pair_close — cursor ────────────────────────────────────────────

    #[test]
    fn auto_close_at_start() {
        // Cursor on 'h' at start of line. Auto-close inserts '(' + ')' before
        // 'h', cursor lands on ')' (the close bracket).
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| insert_pair_close(buf, sels, '(', ')'),
            "(-[)]>hello\n"
        );
    }

    #[test]
    fn auto_close_at_middle() {
        assert_state!(
            "hel-[l]>o\n",
            |(buf, sels)| insert_pair_close(buf, sels, '(', ')'),
            "hel(-[)]>lo\n"
        );
    }

    #[test]
    fn auto_close_before_newline() {
        // Cursor on the structural '\n' — valid insert position.
        assert_state!(
            "hello-[\n]>",
            |(buf, sels)| insert_pair_close(buf, sels, '(', ')'),
            "hello(-[)]>\n"
        );
    }

    #[test]
    fn auto_close_square_bracket() {
        assert_state!(
            "-[x]>\n",
            |(buf, sels)| insert_pair_close(buf, sels, '[', ']'),
            "[-[]]>x\n"
        );
    }

    #[test]
    fn auto_close_symmetric_quote() {
        assert_state!(
            "-[x]>\n",
            |(buf, sels)| insert_pair_close(buf, sels, '"', '"'),
            "\"-[\"]>x\n"
        );
    }

    #[test]
    fn auto_close_multi_cursor() {
        // Two cursors both get auto-closed independently.
        assert_state!(
            "-[a]>b-[c]>d\n",
            |(buf, sels)| insert_pair_close(buf, sels, '(', ')'),
            "(-[)]>ab(-[)]>cd\n"
        );
    }

    // ── insert_pair_close — wrap selection ────────────────────────────────────

    #[test]
    fn wrap_forward_selection() {
        assert_state!(
            "-[hello]>\n",
            |(buf, sels)| insert_pair_close(buf, sels, '(', ')'),
            "(hello-[)]>\n"
        );
    }

    #[test]
    fn wrap_backward_selection() {
        assert_state!(
            "<[hello]-\n",
            |(buf, sels)| insert_pair_close(buf, sels, '(', ')'),
            "(hello-[)]>\n"
        );
    }

    #[test]
    fn wrap_partial_word() {
        assert_state!(
            "foo -[bar]> baz\n",
            |(buf, sels)| insert_pair_close(buf, sels, '[', ']'),
            "foo [bar-[]]> baz\n"
        );
    }

    #[test]
    fn wrap_multi_cursor_selections() {
        assert_state!(
            "-[ab]>c-[de]>f\n",
            |(buf, sels)| insert_pair_close(buf, sels, '(', ')'),
            "(ab-[)]>c(de-[)]>f\n"
        );
    }

    // ── delete_pair ───────────────────────────────────────────────────────────

    #[test]
    fn delete_pair_parens() {
        // Text: `(|)` where cursor is on `)`. Both are deleted.
        assert_state!(
            "(-[)]>\n",
            |(buf, sels)| delete_pair(buf, sels),
            "-[\n]>"
        );
    }

    #[test]
    fn delete_pair_inside_word() {
        assert_state!(
            "foo(-[)]>bar\n",
            |(buf, sels)| delete_pair(buf, sels),
            "foo-[b]>ar\n"
        );
    }

    #[test]
    fn delete_pair_square() {
        assert_state!(
            "[-[]]>\n",
            |(buf, sels)| delete_pair(buf, sels),
            "-[\n]>"
        );
    }

    #[test]
    fn delete_pair_quote() {
        assert_state!(
            "\"-[\"]>\n",
            |(buf, sels)| delete_pair(buf, sels),
            "-[\n]>"
        );
    }

    #[test]
    fn delete_pair_multi_cursor() {
        assert_state!(
            "(-[)]>(-[)]>\n",
            |(buf, sels)| delete_pair(buf, sels),
            "-[\n]>"
        );
    }

    // ── should_auto_pair_at ───────────────────────────────────────────────────

    fn default_pairs() -> Vec<Pair> {
        vec![
            Pair { open: '(', close: ')' },
            Pair { open: '[', close: ']' },
            Pair { open: '{', close: '}' },
            Pair { open: '"', close: '"' },
            Pair { open: '\'', close: '\'' },
            Pair { open: '`', close: '`' },
        ]
    }

    fn paren() -> Pair { Pair { open: '(', close: ')' } }
    fn quote() -> Pair { Pair { open: '"', close: '"' } }

    #[test]
    fn auto_pair_next_alphanumeric_rejects_asymmetric() {
        // Cursor at 0, next char 'b' — should NOT auto-pair `(`.
        let buf = Text::from("bar");
        let pairs = default_pairs();
        assert!(!should_auto_pair_at(&buf, 0, &paren(), &pairs));
    }

    #[test]
    fn auto_pair_next_alphanumeric_rejects_symmetric() {
        // Cursor at 0, next char 'b' — should NOT auto-pair `"`.
        let buf = Text::from("bar");
        let pairs = default_pairs();
        assert!(!should_auto_pair_at(&buf, 0, &quote(), &pairs));
    }

    #[test]
    fn auto_pair_next_space_accepts() {
        // Cursor at 4 (space between words) — next char is space.
        let buf = Text::from("foo bar");
        let pairs = default_pairs();
        assert!(should_auto_pair_at(&buf, 3, &paren(), &pairs));
    }

    #[test]
    fn auto_pair_next_newline_accepts() {
        // Cursor on the structural `\n` — next char is newline.
        let buf = Text::from("hello");
        let pairs = default_pairs();
        assert!(should_auto_pair_at(&buf, 5, &paren(), &pairs));
    }

    #[test]
    fn auto_pair_next_closing_bracket_accepts() {
        // Cursor at 1 (inside `()`), next char is `)`.
        let buf = Text::from("()");
        let pairs = default_pairs();
        assert!(should_auto_pair_at(&buf, 1, &paren(), &pairs));
    }

    #[test]
    fn auto_pair_symmetric_prev_alphanumeric_rejects() {
        // `don't` — cursor at 3 (the `'`), prev char is `n`.
        // Should NOT auto-pair the quote.
        let buf = Text::from("don't");
        let pairs = default_pairs();
        assert!(!should_auto_pair_at(&buf, 3, &quote(), &pairs));
    }

    #[test]
    fn auto_pair_symmetric_prev_space_accepts() {
        // `say ` — cursor at 4 (the `\n`), prev char is space.
        let buf = Text::from("say ");
        let pairs = default_pairs();
        assert!(should_auto_pair_at(&buf, 4, &quote(), &pairs));
    }

    #[test]
    fn auto_pair_symmetric_at_position_zero_accepts() {
        // Cursor at 0 in an empty buffer (just the structural `\n`).
        // No prev char and next char is `\n` (whitespace) → should auto-pair.
        let buf = Text::from("");
        let pairs = default_pairs();
        assert!(should_auto_pair_at(&buf, 0, &quote(), &pairs));
    }

    #[test]
    fn auto_pair_symmetric_prev_open_bracket_accepts() {
        // `( ` — cursor at 1 (space), prev char is `(` (not alphanumeric), next is space.
        let buf = Text::from("( foo");
        let pairs = default_pairs();
        assert!(should_auto_pair_at(&buf, 1, &quote(), &pairs));
    }

    #[test]
    fn auto_pair_asymmetric_ignores_prev_word_char() {
        // `x ` — cursor at 1 (space), prev is `x`. Parens are asymmetric so
        // only the next-char rule applies; next is space → accept.
        let buf = Text::from("x foo");
        let pairs = default_pairs();
        assert!(should_auto_pair_at(&buf, 1, &paren(), &pairs));
    }
}
