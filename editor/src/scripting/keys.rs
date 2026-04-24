//! Parse human-readable key-sequence strings into `Vec<KeyEvent>`.
//!
//! Used both by the `bind-key!` / `bind-wait-char!` builtins at eval time and
//! by [`crate::scripting`]'s ledger restoration path when replaying a plugin's
//! prior bindings.  Keeping it as a standalone module avoids the layer
//! violation of having `scripting/mod.rs` reach into `scripting/builtins/`.
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
        "enter" | "return" | "cr" => return Ok(KeyCode::Enter),
        "esc" | "escape" => return Ok(KeyCode::Esc),
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
        // In the new format, < and > are just characters — no special quoting needed.
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
}
