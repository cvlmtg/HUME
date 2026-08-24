use super::*;
use hume_test_fixtures::assert_state;

// ── insert_pair_close — cursor ────────────────────────────────────────────

#[test]
fn auto_close_at_start() {
    // Cursor on 'h' at start of line. Auto-close inserts '(' + ')' before
    // 'h', cursor lands on ')' (the close bracket).
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| insert_pair_close(text, sels, '(', ')'),
        "(-[)]>hello\n"
    );
}

#[test]
fn auto_close_at_middle() {
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| insert_pair_close(text, sels, '(', ')'),
        "hel(-[)]>lo\n"
    );
}

#[test]
fn auto_close_before_newline() {
    // Cursor on the structural '\n' — valid insert position.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| insert_pair_close(text, sels, '(', ')'),
        "hello(-[)]>\n"
    );
}

#[test]
fn auto_close_square_bracket() {
    assert_state!(
        "-[x]>\n",
        |(text, sels)| insert_pair_close(text, sels, '[', ']'),
        "[-[]]>x\n"
    );
}

#[test]
fn auto_close_symmetric_quote() {
    assert_state!(
        "-[x]>\n",
        |(text, sels)| insert_pair_close(text, sels, '"', '"'),
        "\"-[\"]>x\n"
    );
}

#[test]
fn auto_close_multi_cursor() {
    // Two cursors both get auto-closed independently.
    assert_state!(
        "-[a]>b-[c]>d\n",
        |(text, sels)| insert_pair_close(text, sels, '(', ')'),
        "(-[)]>ab(-[)]>cd\n"
    );
}

// ── delete_pair ───────────────────────────────────────────────────────────

#[test]
fn delete_pair_parens() {
    // Text: `(|)` where cursor is on `)`. Both are deleted.
    assert_state!("(-[)]>\n", |(text, sels)| delete_pair(text, sels), "-[\n]>");
}

#[test]
fn delete_pair_inside_word() {
    assert_state!(
        "foo(-[)]>bar\n",
        |(text, sels)| delete_pair(text, sels),
        "foo-[b]>ar\n"
    );
}

#[test]
fn delete_pair_square() {
    assert_state!("[-[]]>\n", |(text, sels)| delete_pair(text, sels), "-[\n]>");
}

#[test]
fn delete_pair_quote() {
    assert_state!(
        "\"-[\"]>\n",
        |(text, sels)| delete_pair(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_pair_multi_cursor() {
    assert_state!(
        "(-[)]>(-[)]>\n",
        |(text, sels)| delete_pair(text, sels),
        "-[\n]>"
    );
}

// ── should_auto_pair_at ───────────────────────────────────────────────────

fn default_pairs() -> Vec<Pair> {
    vec![
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
    ]
}

fn paren() -> Pair {
    Pair {
        open: '(',
        close: ')',
    }
}
fn quote() -> Pair {
    Pair {
        open: '"',
        close: '"',
    }
}

#[test]
fn auto_pair_next_alphanumeric_rejects_asymmetric() {
    // Cursor at 0, next char 'b' — should NOT auto-pair `(`.
    let text = BufferText::from("bar");
    let pairs = default_pairs();
    assert!(!should_auto_pair_at(&text, 0, &paren(), &pairs));
}

#[test]
fn auto_pair_next_alphanumeric_rejects_symmetric() {
    // Cursor at 0, next char 'b' — should NOT auto-pair `"`.
    let text = BufferText::from("bar");
    let pairs = default_pairs();
    assert!(!should_auto_pair_at(&text, 0, &quote(), &pairs));
}

#[test]
fn auto_pair_next_space_accepts() {
    // Cursor at 4 (space between words) — next char is space.
    let text = BufferText::from("foo bar");
    let pairs = default_pairs();
    assert!(should_auto_pair_at(&text, 3, &paren(), &pairs));
}

#[test]
fn auto_pair_next_newline_accepts() {
    // Cursor on the structural `\n` — next char is newline.
    let text = BufferText::from("hello");
    let pairs = default_pairs();
    assert!(should_auto_pair_at(&text, 5, &paren(), &pairs));
}

#[test]
fn auto_pair_next_closing_bracket_accepts() {
    // Cursor at 1 (inside `()`), next char is `)`.
    let text = BufferText::from("()");
    let pairs = default_pairs();
    assert!(should_auto_pair_at(&text, 1, &paren(), &pairs));
}

#[test]
fn auto_pair_symmetric_prev_alphanumeric_rejects() {
    // `don't` — cursor at 3 (the `'`), prev char is `n`.
    // Should NOT auto-pair the quote.
    let text = BufferText::from("don't");
    let pairs = default_pairs();
    assert!(!should_auto_pair_at(&text, 3, &quote(), &pairs));
}

#[test]
fn auto_pair_symmetric_prev_space_accepts() {
    // `say ` — cursor at 4 (the `\n`), prev char is space.
    let text = BufferText::from("say ");
    let pairs = default_pairs();
    assert!(should_auto_pair_at(&text, 4, &quote(), &pairs));
}

#[test]
fn auto_pair_symmetric_at_position_zero_accepts() {
    // Cursor at 0 in an empty buffer (just the structural `\n`).
    // No prev char and next char is `\n` (whitespace) → should auto-pair.
    let text = BufferText::from("");
    let pairs = default_pairs();
    assert!(should_auto_pair_at(&text, 0, &quote(), &pairs));
}

#[test]
fn auto_pair_symmetric_prev_open_bracket_accepts() {
    // `( ` — cursor at 1 (space), prev char is `(` (not alphanumeric), next is space.
    let text = BufferText::from("( foo");
    let pairs = default_pairs();
    assert!(should_auto_pair_at(&text, 1, &quote(), &pairs));
}

#[test]
fn auto_pair_asymmetric_ignores_prev_word_char() {
    // `x ` — cursor at 1 (space), prev is `x`. Parens are asymmetric so
    // only the next-char rule applies; next is space → accept.
    let text = BufferText::from("x foo");
    let pairs = default_pairs();
    assert!(should_auto_pair_at(&text, 1, &paren(), &pairs));
}
