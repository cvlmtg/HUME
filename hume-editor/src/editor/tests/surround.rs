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

// ── surround-add (`mw`) ───────────────────────────────────────────────────────

#[test]
fn mw_wraps_with_bracket() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}

#[test]
fn mw_wraps_with_brace_via_close_char() {
    // `mw}` should normalize to the pair `{` `}`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('}'));
    assert_eq!(state(&ed), "{bar-[}]>\n");
}

#[test]
fn mw_wraps_symmetric_quote() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"bar-[\"]>\n");
}

#[test]
fn mw_wraps_unknown_char_symmetric() {
    // `*` is not a configured pair — wraps symmetrically open == close == `*`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('*'));
    assert_eq!(state(&ed), "*bar-[*]>\n");
}

#[test]
fn mw_wraps_multi_cursor() {
    let mut ed = editor_from("-[ab]>c-[de]>f\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(ab-[)]>c(de-[)]>f\n");
}

#[test]
fn mw_wraps_cursor_one_char() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[h-[]]>ello\n");
}

#[test]
fn mw_esc_cancels() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key_esc()); // cancel before typing the delimiter
    assert_eq!(state(&ed), "-[bar]>\n");
}

#[test]
fn mw_wraps_when_auto_pairs_disabled() {
    // surround-add uses the pairs table only as a lookup; it ignores the
    // auto-pairs-enabled flag. `mw[` must still wrap even when auto-pairs are off.
    let mut ed = editor_from("-[bar]>\n");
    ed.state.settings.auto_pairs_enabled = false;
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}
