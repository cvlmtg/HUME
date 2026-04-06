use crate::core::buffer::Buffer;
use crate::core::changeset::ChangeSet;
use crate::ops::edit::apply_edit;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::core::selection::{Selection, SelectionSet};

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

/// Configuration for automatic bracket and quote pairing.
///
/// The `enabled` flag is a master switch; `pairs` controls which characters
/// participate. Both are configurable via the Steel scripting layer.
#[derive(Debug, Clone)]
pub(crate) struct AutoPairsConfig {
    /// Master switch. When `false`, all auto-pair behavior is disabled and
    /// the editor falls back to plain `insert_char` / `delete_char_backward`.
    pub enabled: bool,
    /// The active pairs. Lookup is by character; order does not matter.
    pub pairs: Vec<Pair>,
}

impl Default for AutoPairsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            pairs: vec![
                Pair { open: '(', close: ')' },
                Pair { open: '[', close: ']' },
                Pair { open: '{', close: '}' },
                Pair { open: '"', close: '"' },
                Pair { open: '\'', close: '\'' },
                Pair { open: '`', close: '`' },
            ],
        }
    }
}

impl AutoPairsConfig {
    /// Return the pair whose `open` char matches `ch`, if any.
    ///
    /// Works for both asymmetric pairs (`(` → `Pair { '(', ')' }`) and
    /// symmetric pairs (`"` → `Pair { '"', '"' }`).
    pub(crate) fn pair_for_open(&self, ch: char) -> Option<&Pair> {
        self.pairs.iter().find(|p| p.open == ch)
    }

    /// Return the pair whose `close` char matches `ch`, if any.
    ///
    /// Excludes symmetric pairs (where `open == close`) because a lone `"` is
    /// always treated as an opener — skip-close for symmetric pairs is handled
    /// in the caller via `pair_for_open` + `is_symmetric` check.
    pub(crate) fn pair_for_close(&self, ch: char) -> Option<&Pair> {
        self.pairs.iter().find(|p| p.close == ch && !p.is_symmetric())
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
    buf: Buffer,
    sels: SelectionSet,
    open: char,
    close: char,
) -> (Buffer, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start - b.old_pos());

        if sel.is_collapsed() {
            // Simple auto-close: insert open + close.
            b.insert_char(open);
            b.insert_char(close);
            // Cursor on `close` (new_pos is one past close, step back by 1).
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
            // Cursor on the close bracket.
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
    buf: Buffer,
    sels: SelectionSet,
) -> (Buffer, SelectionSet, ChangeSet) {
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
        // Buffer: `(|)` where cursor is on `)`. Both are deleted.
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
}
