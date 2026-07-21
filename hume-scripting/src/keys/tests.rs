use super::*;

fn parse(s: &str) -> Result<Vec<KeyEvent>, String> {
    parse_key_sequence(s)
}

fn stream(s: &str) -> Result<Vec<KeyEvent>, String> {
    parse_key_stream(s)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, Modifiers::NONE)
}
fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, Modifiers::CONTROL)
}
fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, Modifiers::SHIFT)
}
fn alt(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, Modifiers::ALT)
}

#[test]
fn single_char() {
    assert_eq!(parse("f").unwrap(), vec![key(KeyCode::Char('f'))]);
    assert_eq!(parse("g").unwrap(), vec![key(KeyCode::Char('g'))]);
}

#[test]
fn uppercase_char_preserved() {
    assert_eq!(parse("G").unwrap(), vec![key(KeyCode::Char('G'))]);
}

#[test]
fn multi_key_sequence() {
    assert_eq!(
        parse("g h").unwrap(),
        vec![key(KeyCode::Char('g')), key(KeyCode::Char('h'))],
    );
}

#[test]
fn three_key_sequence() {
    assert_eq!(
        parse("m a w").unwrap(),
        vec![
            key(KeyCode::Char('m')),
            key(KeyCode::Char('a')),
            key(KeyCode::Char('w')),
        ],
    );
}

#[test]
fn named_key_esc() {
    assert_eq!(parse("esc").unwrap(), vec![key(KeyCode::Escape)]);
    assert_eq!(parse("escape").unwrap(), vec![key(KeyCode::Escape)]);
}

#[test]
fn named_key_enter() {
    assert_eq!(parse("enter").unwrap(), vec![key(KeyCode::Enter)]);
    assert_eq!(parse("cr").unwrap(), vec![key(KeyCode::Enter)]);
}

#[test]
fn named_key_space() {
    assert_eq!(parse("space").unwrap(), vec![key(KeyCode::Char(' '))]);
}

#[test]
fn named_key_backspace() {
    assert_eq!(parse("backspace").unwrap(), vec![key(KeyCode::Backspace)]);
    assert_eq!(parse("bs").unwrap(), vec![key(KeyCode::Backspace)]);
}

#[test]
fn named_key_arrows() {
    assert_eq!(parse("up").unwrap(), vec![key(KeyCode::Up)]);
    assert_eq!(parse("down").unwrap(), vec![key(KeyCode::Down)]);
    assert_eq!(parse("left").unwrap(), vec![key(KeyCode::Left)]);
    assert_eq!(parse("right").unwrap(), vec![key(KeyCode::Right)]);
}

#[test]
fn function_keys() {
    assert_eq!(parse("f1").unwrap(), vec![key(KeyCode::Function(1))]);
    assert_eq!(parse("f12").unwrap(), vec![key(KeyCode::Function(12))]);
}

#[test]
fn f_key_out_of_range_errors() {
    assert!(parse("f0").is_err());
    assert!(parse("f13").is_err());
}

#[test]
fn ctrl_modifier() {
    assert_eq!(parse("ctrl-x").unwrap(), vec![ctrl(KeyCode::Char('x'))]);
    assert_eq!(parse("ctrl-d").unwrap(), vec![ctrl(KeyCode::Char('d'))]);
}

#[test]
fn shift_modifier() {
    assert_eq!(parse("shift-k").unwrap(), vec![shift(KeyCode::Char('k'))]);
}

#[test]
fn alt_modifier() {
    assert_eq!(parse("alt-b").unwrap(), vec![alt(KeyCode::Char('b'))]);
}

#[test]
fn ctrl_shift_combo() {
    let expected = vec![KeyEvent::new(
        KeyCode::Char('k'),
        Modifiers::CONTROL | Modifiers::SHIFT,
    )];
    assert_eq!(parse("ctrl-shift-k").unwrap(), expected);
}

#[test]
fn shift_tab_normalises_to_backtab() {
    assert_eq!(parse("shift-tab").unwrap(), vec![shift(KeyCode::BackTab)],);
}

#[test]
fn angle_brackets_are_plain_chars() {
    // < and > are just characters — no special quoting needed.
    assert_eq!(parse("<").unwrap(), vec![key(KeyCode::Char('<'))]);
    assert_eq!(parse(">").unwrap(), vec![key(KeyCode::Char('>'))]);
}

#[test]
fn mixed_named_and_char_sequence() {
    assert_eq!(
        parse("g esc").unwrap(),
        vec![key(KeyCode::Char('g')), key(KeyCode::Escape)],
    );
}

#[test]
fn unknown_key_errors() {
    assert!(parse("boguskey").is_err());
}

#[test]
fn bare_modifier_prefix_errors() {
    assert!(parse("ctrl-").is_err());
    assert!(parse("shift-").is_err());
}

#[test]
fn empty_sequence_errors() {
    assert!(parse("").is_err());
    assert!(parse("   ").is_err());
}

#[test]
fn ret_alias() {
    assert_eq!(parse("ret").unwrap(), vec![key(KeyCode::Enter)]);
}

#[test]
fn lt_alias() {
    assert_eq!(parse("lt").unwrap(), vec![key(KeyCode::Char('<'))]);
}

// ── parse_key_stream tests ────────────────────────────────────────────────

#[test]
fn stream_single_char() {
    assert_eq!(stream("i").unwrap(), vec![key(KeyCode::Char('i'))]);
}

#[test]
fn stream_multi_char() {
    assert_eq!(
        stream("wbc").unwrap(),
        vec![
            key(KeyCode::Char('w')),
            key(KeyCode::Char('b')),
            key(KeyCode::Char('c')),
        ],
    );
}

#[test]
fn stream_named_key_in_brackets() {
    assert_eq!(
        stream("a<esc>b").unwrap(),
        vec![
            key(KeyCode::Char('a')),
            key(KeyCode::Escape),
            key(KeyCode::Char('b')),
        ],
    );
}

#[test]
fn stream_ret_alias_in_brackets() {
    assert_eq!(stream("<ret>").unwrap(), vec![key(KeyCode::Enter)]);
    assert_eq!(stream("<enter>").unwrap(), vec![key(KeyCode::Enter)]);
}

#[test]
fn stream_lt_escape() {
    assert_eq!(stream("<lt>").unwrap(), vec![key(KeyCode::Char('<'))]);
}

#[test]
fn stream_space_is_literal() {
    // Space in a stream is Char(' '), unlike parse_key_sequence where it separates tokens.
    assert_eq!(
        stream("a b").unwrap(),
        vec![
            key(KeyCode::Char('a')),
            key(KeyCode::Char(' ')),
            key(KeyCode::Char('b')),
        ],
    );
}

#[test]
fn stream_short_ctrl_modifier() {
    assert_eq!(stream("<c-a>").unwrap(), vec![ctrl(KeyCode::Char('a'))]);
    assert_eq!(stream("<c-x>").unwrap(), vec![ctrl(KeyCode::Char('x'))]);
}

#[test]
fn stream_short_alt_modifier() {
    assert_eq!(stream("<a-b>").unwrap(), vec![alt(KeyCode::Char('b'))]);
}

#[test]
fn stream_short_shift_modifier() {
    assert_eq!(stream("<s-tab>").unwrap(), vec![shift(KeyCode::BackTab)]);
}

#[test]
fn stream_long_modifier_still_works() {
    assert_eq!(stream("<ctrl-x>").unwrap(), vec![ctrl(KeyCode::Char('x'))]);
    assert_eq!(stream("<alt-b>").unwrap(), vec![alt(KeyCode::Char('b'))]);
}

#[test]
fn stream_uppercase_literal_char() {
    assert_eq!(stream("G").unwrap(), vec![key(KeyCode::Char('G'))]);
}

#[test]
fn stream_uppercase_in_brackets() {
    // <c-X> → Ctrl+X (uppercase preserved in key name).
    assert_eq!(stream("<c-X>").unwrap(), vec![ctrl(KeyCode::Char('X'))]);
}

#[test]
fn stream_unclosed_bracket_errors() {
    assert!(stream("<esc").is_err());
    assert!(stream("a<b").is_err());
}

#[test]
fn stream_empty_is_ok() {
    assert_eq!(stream("").unwrap(), vec![]);
}
