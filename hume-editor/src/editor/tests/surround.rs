use super::*;
use pretty_assertions::assert_eq;

// ── Surround operations ──────────────────────────────────────────────────────

/// `ms(` selects the surrounding `(` and `)` as two cursor selections.
#[test]
fn surround_select_paren() {
    let mut ed = editor_from("(-[h]>ello)\n");
    for ch in "ms(".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "-[(]>hello-[)]>\n");
}

/// `ms[` works the same for square brackets.
#[test]
fn surround_select_bracket() {
    let mut ed = editor_from("[-[h]>ello]\n");
    for ch in "ms[".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "-[[]>hello-[]]>\n");
}

/// `ms"` selects surrounding double quotes.
#[test]
fn surround_select_double_quote() {
    let mut ed = editor_from("\"-[h]>ello\"\n");
    for ch in "ms\"".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "-[\"]>hello-[\"]>\n");
}

/// `ms(` → `d` deletes the surrounding parens, leaving two cursors.
#[test]
fn surround_delete_paren() {
    let mut ed = editor_from("(-[h]>ello)\n");
    for ch in "ms(".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key('d'));
    // Two cursors remain: one where `(` was (now `h`), one where `)` was
    // (now the structural `\n`).
    assert_eq!(state(&ed), "-[h]>ello-[\n]>");
}

/// `ms(` → `r[` replaces `()` with `[]` via smart replace.
#[test]
fn surround_replace_paren_with_bracket() {
    let mut ed = editor_from("(-[h]>ello)\n");
    for ch in "ms(".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key('r'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "-[[]>hello-[]]>\n");
}

/// `ms"` → `r(` replaces `""` with `()` (symmetric → asymmetric).
#[test]
fn surround_replace_quote_with_paren() {
    let mut ed = editor_from("\"-[h]>ello\"\n");
    for ch in "ms\"".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key('r'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "-[(]>hello-[)]>\n");
}

/// `ms(` with no enclosing parens is a no-op.
#[test]
fn surround_no_match_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in "ms(".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "-[h]>ello\n");
}
