//! Parse human-readable key-sequence strings into `Vec<KeyEvent>`.
//!
//! Used by the `bind-key!` / `bind-wait-char!` builtins at eval time and by
//! the `:bind` typed command.  Keeping it as a standalone
//! module avoids the layer violation of having `scripting/mod.rs` reach into
//! `scripting/builtins/`.
//!
//! ## Format
//!
//! A key string is a whitespace-separated list of key tokens; each token has
//! the form `[modifier-]* key_name`.
//!
//! - Modifiers: `ctrl-`, `shift-`, `alt-` (case-insensitive; order doesn't matter)
//! - Named keys: `esc`, `tab`, `enter`, `space`, `backspace`, `delete`, `insert`,
//!   `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, `f1`–`f12`
//! - Single character: any single Unicode character (e.g. `f`, `G`, `<`, `>`)
//! - Multi-key sequences: space-separated tokens, e.g. `"g h"`, `"m d"`
//!
//! ## Examples
//!
//! ```text
//! "f"         → [Char('f')]
//! "G"         → [Char('G')]
//! "ctrl-x"    → [Char('x') | CONTROL]
//! "shift-tab" → [BackTab | SHIFT]
//! "esc"       → [Esc]
//! "g h"       → [Char('g'), Char('h')]
//! "m d"       → [Char('m'), Char('d')]
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Parse a key-sequence string into a `Vec<KeyEvent>`.
///
/// The string is a whitespace-separated list of key tokens.  Each token has
/// the form `[modifier-]* key_name` where modifiers are `ctrl-`, `shift-`,
/// or `alt-` (case-insensitive) and `key_name` is either a named key or a
/// single Unicode character.
///
/// Returns an error string if the sequence is empty or any token is
/// unrecognised.
pub(crate) fn parse_key_sequence(s: &str) -> Result<Vec<KeyEvent>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("key sequence must not be empty".to_string());
    }
    s.split_whitespace().map(parse_single_key).collect()
}

/// Parse a single key token (no spaces) into a [`KeyEvent`].
fn parse_single_key(token: &str) -> Result<KeyEvent, String> {
    // Lowercase a copy for modifier-prefix stripping; key name is recovered
    // from the original token so that single-char case is preserved
    // ("G" stays 'G', not 'g').
    let lower = token.to_ascii_lowercase();
    let mut modifiers = KeyModifiers::NONE;
    let mut rest_lower = lower.as_str();

    // Strip all modifier prefixes — order doesn't matter.
    loop {
        if let Some(tail) = rest_lower.strip_prefix("ctrl-") {
            modifiers |= KeyModifiers::CONTROL;
            rest_lower = tail;
        } else if let Some(tail) = rest_lower.strip_prefix("shift-") {
            modifiers |= KeyModifiers::SHIFT;
            rest_lower = tail;
        } else if let Some(tail) = rest_lower.strip_prefix("alt-") {
            modifiers |= KeyModifiers::ALT;
            rest_lower = tail;
        } else {
            break;
        }
    }

    // Recover the original-case key name by measuring how many bytes the
    // modifier prefixes consumed from the start of `token`.
    let consumed = token.len() - rest_lower.len();
    let key_name = &token[consumed..];

    if key_name.is_empty() {
        return Err(format!(
            "key token '{token}' has no key name after modifiers"
        ));
    }

    let code = parse_key_code(key_name)?;
    // `shift-tab` is conventionally represented as BackTab in crossterm.
    let (code, modifiers) = normalise_shift_tab(code, modifiers);
    Ok(KeyEvent::new(code, modifiers))
}

/// Map a bare key name to a [`KeyCode`].
///
/// Named keys are matched case-insensitively via the already-lowercased
/// `key_name`.  Single-character keys preserve the original case so that
/// `"G"` → `Char('G')` and `"g"` → `Char('g')` remain distinct.
fn parse_key_code(key_name: &str) -> Result<KeyCode, String> {
    let lower = key_name.to_ascii_lowercase();
    match lower.as_str() {
        "space" => return Ok(KeyCode::Char(' ')),
        "tab" => return Ok(KeyCode::Tab),
        "enter" | "return" | "cr" | "ret" => return Ok(KeyCode::Enter),
        "esc" | "escape" => return Ok(KeyCode::Esc),
        "lt" => return Ok(KeyCode::Char('<')),
        "backspace" | "bs" => return Ok(KeyCode::Backspace),
        "delete" | "del" => return Ok(KeyCode::Delete),
        "insert" | "ins" => return Ok(KeyCode::Insert),
        "home" => return Ok(KeyCode::Home),
        "end" => return Ok(KeyCode::End),
        "pageup" => return Ok(KeyCode::PageUp),
        "pagedown" => return Ok(KeyCode::PageDown),
        "up" => return Ok(KeyCode::Up),
        "down" => return Ok(KeyCode::Down),
        "left" => return Ok(KeyCode::Left),
        "right" => return Ok(KeyCode::Right),
        _ => {}
    }

    // F-keys: f1 … f12.
    if let Some(n) = lower
        .strip_prefix('f')
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|&n| (1..=12).contains(&n))
    {
        return Ok(KeyCode::F(n));
    }

    // Single Unicode character — must be exactly one char.
    let mut chars = key_name.chars();
    let Some(ch) = chars.next() else {
        return Err("key name is empty after modifiers".to_string());
    };
    if chars.next().is_some() {
        return Err(format!("unrecognised key name '{key_name}'"));
    }
    Ok(KeyCode::Char(ch))
}

/// Parse a continuous golf-style key stream into a `Vec<KeyEvent>`.
///
/// Unlike [`parse_key_sequence`] (whitespace-separated tokens where space is a
/// separator), this format is a raw stream where every character is a
/// keystroke.  Space is a literal `Char(' ')`.
///
/// ## Format
///
/// - Bare printable characters → `Char(c)` each.
/// - `<name>` → named or modified key, using the same names as
///   [`parse_key_sequence`] plus the following additions and shorthands:
///   - `<ret>` → Enter (alias for `<enter>` / `<cr>`).
///   - `<lt>` → literal `<`.
///   - Short modifier prefixes: `c-` (Ctrl), `a-` (Alt), `s-` (Shift).
///   - Long forms also accepted: `ctrl-`, `alt-`, `shift-`.
///
/// ## Examples
///
/// ```text
/// "i"           → [Char('i')]
/// "wbc<esc>"    → [Char('w'), Char('b'), Char('c'), Esc]
/// "<c-a>"       → [Char('a') | CONTROL]
/// "a b"         → [Char('a'), Char(' '), Char('b')]   (space is literal)
/// "<lt>"        → [Char('<')]
/// "<ret>"       → [Enter]
/// ```
pub fn parse_key_stream(s: &str) -> Result<Vec<KeyEvent>, String> {
    let mut keys = Vec::new();
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < s.len() {
        if bytes[i] == b'<' {
            // Scan forward for the closing '>'.  Both '<' and '>' are ASCII
            // (one byte), so byte indexing into the str slice is safe here.
            let start = i + 1;
            let close = bytes[start..]
                .iter()
                .position(|&b| b == b'>')
                .ok_or_else(|| format!("unclosed '<' at position {i}"))?;
            let inner = &s[start..start + close];
            keys.push(parse_stream_token(inner)?);
            i = start + close + 1; // advance past '>'
        } else {
            // Literal character — may be multi-byte UTF-8.
            let ch = s[i..]
                .chars()
                .next()
                .expect("i is on a char boundary: advanced by len_utf8 each iteration");
            keys.push(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            i += ch.len_utf8();
        }
    }
    Ok(keys)
}

/// Parse the content inside `<…>` from a golf key stream.
///
/// Expands the short modifier prefixes `c-`, `a-`, `s-` to the long forms
/// (`ctrl-`, `alt-`, `shift-`) that [`parse_single_key`] understands, then
/// delegates.  `<lt>` is handled before expansion as it would otherwise be
/// mistaken for a single-char key.
fn parse_stream_token(inner: &str) -> Result<KeyEvent, String> {
    // `<lt>` is the self-escape for a literal '<'.  Handle it first because
    // it is two characters and would fall through to parse_single_key as an
    // unknown name otherwise.
    if inner.eq_ignore_ascii_case("lt") {
        return Ok(KeyEvent::new(KeyCode::Char('<'), KeyModifiers::NONE));
    }
    let expanded = expand_short_modifiers(inner);
    parse_single_key(&expanded)
}

/// Expand short modifier prefixes used in golf `<…>` notation to the long
/// forms that [`parse_single_key`] expects.
///
/// `c-` → `ctrl-`, `a-` → `alt-`, `s-` → `shift-`.  The key-name portion
/// (everything after all modifiers) is preserved with its original case so
/// that `<c-X>` maps to `Ctrl+X` rather than `Ctrl+x`.
fn expand_short_modifiers(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    let mut lo = lower.as_str();
    let mut orig = token;
    let mut result = String::new();

    loop {
        if lo.starts_with("c-") {
            result.push_str("ctrl-");
            lo = &lo[2..];
            orig = &orig[2..];
        } else if lo.starts_with("a-") {
            result.push_str("alt-");
            lo = &lo[2..];
            orig = &orig[2..];
        } else if lo.starts_with("s-") {
            result.push_str("shift-");
            lo = &lo[2..];
            orig = &orig[2..];
        } else {
            break;
        }
    }
    result.push_str(orig);
    result
}

/// Crossterm uses `BackTab` (not `Tab | SHIFT`) for Shift+Tab.
fn normalise_shift_tab(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    if code == KeyCode::Tab && mods.contains(KeyModifiers::SHIFT) {
        (KeyCode::BackTab, mods)
    } else {
        (code, mods)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Vec<KeyEvent>, String> {
        parse_key_sequence(s)
    }

    fn stream(s: &str) -> Result<Vec<KeyEvent>, String> {
        parse_key_stream(s)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }
    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }
    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
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
        assert_eq!(parse("esc").unwrap(), vec![key(KeyCode::Esc)]);
        assert_eq!(parse("escape").unwrap(), vec![key(KeyCode::Esc)]);
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
        assert_eq!(parse("f1").unwrap(), vec![key(KeyCode::F(1))]);
        assert_eq!(parse("f12").unwrap(), vec![key(KeyCode::F(12))]);
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
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
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
            vec![key(KeyCode::Char('g')), key(KeyCode::Esc)],
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
                key(KeyCode::Esc),
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
}
