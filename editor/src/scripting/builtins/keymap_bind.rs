//! `(bind-key! mode key-sequence command-name)` builtin and key-string parser.
//!
//! Key strings use Helix-inspired notation:
//! - Single printable character: `"f"` → Char('f')
//! - Special names wrapped in `<…>`: `"<esc>"`, `"<ctrl-d>"`, `"<tab>"`, etc.
//! - Multi-key sequences: `"gd"` → [g, d]; `"g<esc>"` → [g, Esc]
//!
//! The full sequence is produced left-to-right, mixing plain chars and
//! angle-bracket tokens freely.

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use crate::editor::keymap::BindMode;
use crate::scripting::ledger::Owner;

type SteelResult = Result<SteelVal, SteelErr>;

// ── Key string parser ─────────────────────────────────────────────────────────

/// Parse a key-sequence string like `"f"`, `"gd"`, `"<ctrl-d>"`, or
/// `"g<esc>"` into a `Vec<KeyEvent>`.
///
/// Returns an error string on unrecognised tokens.
fn parse_key_sequence(s: &str) -> Result<Vec<KeyEvent>, String> {
    let mut keys = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' {
            // Consume up to the matching '>'.
            let mut token = String::new();
            let mut closed = false;
            for ch in chars.by_ref() {
                if ch == '>' {
                    closed = true;
                    break;
                }
                token.push(ch);
            }
            if !closed {
                return Err(format!("unclosed '<' in key sequence '{s}'"));
            }
            keys.push(parse_angle_token(&token, s)?);
        } else {
            keys.push(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    if keys.is_empty() {
        return Err(format!("empty key sequence"));
    }
    Ok(keys)
}

/// Parse the content of a `<…>` token (the text between `<` and `>`).
fn parse_angle_token(token: &str, full_seq: &str) -> Result<KeyEvent, String> {
    let lower = token.to_ascii_lowercase();

    // ctrl-X: modifer + key
    if let Some(rest) = lower.strip_prefix("ctrl-") {
        let code = simple_key_code(rest, full_seq)?;
        // For Ctrl+char, crossterm expects the lowercase character.
        return Ok(KeyEvent::new(code, KeyModifiers::CONTROL));
    }
    if let Some(rest) = lower.strip_prefix("shift-") {
        let code = simple_key_code(rest, full_seq)?;
        return Ok(KeyEvent::new(code, KeyModifiers::SHIFT));
    }
    if let Some(rest) = lower.strip_prefix("alt-") {
        let code = simple_key_code(rest, full_seq)?;
        return Ok(KeyEvent::new(code, KeyModifiers::ALT));
    }

    // Plain special key
    let code = simple_key_code(&lower, full_seq)?;
    Ok(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Map a lowercase key name to a `KeyCode`.
fn simple_key_code(name: &str, full_seq: &str) -> Result<KeyCode, String> {
    match name {
        "backspace" | "bs"        => Ok(KeyCode::Backspace),
        "enter" | "ret" | "cr"    => Ok(KeyCode::Enter),
        "left"                    => Ok(KeyCode::Left),
        "right"                   => Ok(KeyCode::Right),
        "up"                      => Ok(KeyCode::Up),
        "down"                    => Ok(KeyCode::Down),
        "home"                    => Ok(KeyCode::Home),
        "end"                     => Ok(KeyCode::End),
        "pageup" | "pgup"         => Ok(KeyCode::PageUp),
        "pagedown" | "pgdown"     => Ok(KeyCode::PageDown),
        "tab"                     => Ok(KeyCode::Tab),
        "backtab"                 => Ok(KeyCode::BackTab),
        "delete" | "del"          => Ok(KeyCode::Delete),
        "insert" | "ins"          => Ok(KeyCode::Insert),
        "esc" | "escape"          => Ok(KeyCode::Esc),
        "space"                   => Ok(KeyCode::Char(' ')),
        "lt"                      => Ok(KeyCode::Char('<')),
        "gt"                      => Ok(KeyCode::Char('>')),
        "f1"  => Ok(KeyCode::F(1)),  "f2"  => Ok(KeyCode::F(2)),
        "f3"  => Ok(KeyCode::F(3)),  "f4"  => Ok(KeyCode::F(4)),
        "f5"  => Ok(KeyCode::F(5)),  "f6"  => Ok(KeyCode::F(6)),
        "f7"  => Ok(KeyCode::F(7)),  "f8"  => Ok(KeyCode::F(8)),
        "f9"  => Ok(KeyCode::F(9)),  "f10" => Ok(KeyCode::F(10)),
        "f11" => Ok(KeyCode::F(11)), "f12" => Ok(KeyCode::F(12)),
        _ => {
            // Single character
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(KeyCode::Char(c)),
                _ => Err(format!("unknown key '<{name}>' in sequence '{full_seq}'")),
            }
        }
    }
}

// ── Builtin ───────────────────────────────────────────────────────────────────

/// `(bind-key! mode key-sequence command-name)`
///
/// Binds a key sequence in the given mode to a named command.
///
/// - `mode` — `"normal"`, `"extend"`, or `"insert"` (case-insensitive).
/// - `key-sequence` — a string parsed by [`parse_key_sequence`].
/// - `command-name` — the canonical command name (must be registered in
///   the [`CommandRegistry`] at dispatch time; not validated here).
///
/// Records a ledger entry when called from a plugin body.
pub(crate) fn bind_key(args: &[SteelVal]) -> SteelResult {
    if args.len() != 3 {
        steel::stop!(ArityMismatch =>
            "bind-key! expects 3 args (mode key-sequence command-name), got {}", args.len());
    }

    let mode_str = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "bind-key!: first arg (mode) must be a string, got {:?}", args[0]),
    };
    let key_str = match &args[1] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "bind-key!: second arg (key-sequence) must be a string, got {:?}", args[1]),
    };
    let cmd_name = match &args[2] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "bind-key!: third arg (command-name) must be a string, got {:?}", args[2]),
    };

    let mode = match mode_str.to_ascii_lowercase().as_str() {
        "normal" => BindMode::Normal,
        "extend" => BindMode::Extend,
        "insert" => BindMode::Insert,
        _ => steel::stop!(Generic =>
            "bind-key!: unknown mode '{}'; expected normal, extend, or insert", mode_str),
    };

    let keys = parse_key_sequence(&key_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;

    super::with_ctx(|ctx| {
        let prior_owner = ctx.plugin_stack.current_owner();

        ctx.keymap.bind_user(mode, &keys, Cow::Owned(cmd_name));

        // Record ledger entry for plugin-attributed mutations.
        if let Owner::Plugin(ref plugin_id) = prior_owner {
            // Use the key sequence string as the ledger key so the ledger can
            // identify which binding to restore.  The prior value for a keybind
            // is stored as an empty string here — Phase 3 records the fact of
            // the mutation; full keybind restoration (with prior-value) is
            // implemented in Phase 3b alongside plugin unload.
            ctx.ledger_stack.record(
                plugin_id,
                key_str,
                prior_owner.clone(),
                String::new(), // prior keybind value — restored in Phase 3b
            );
        }

        Ok(SteelVal::Void)
    })
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

    #[test]
    fn single_char() {
        assert_eq!(parse("f").unwrap(), vec![key(KeyCode::Char('f'))]);
        assert_eq!(parse("g").unwrap(), vec![key(KeyCode::Char('g'))]);
    }

    #[test]
    fn multi_char_sequence() {
        assert_eq!(
            parse("gd").unwrap(),
            vec![key(KeyCode::Char('g')), key(KeyCode::Char('d'))],
        );
    }

    #[test]
    fn special_key_esc() {
        assert_eq!(parse("<esc>").unwrap(), vec![key(KeyCode::Esc)]);
    }

    #[test]
    fn ctrl_modifier() {
        assert_eq!(parse("<ctrl-d>").unwrap(), vec![ctrl(KeyCode::Char('d'))]);
    }

    #[test]
    fn mixed_sequence() {
        assert_eq!(
            parse("g<esc>").unwrap(),
            vec![key(KeyCode::Char('g')), key(KeyCode::Esc)],
        );
    }

    #[test]
    fn space_key() {
        assert_eq!(parse("<space>").unwrap(), vec![key(KeyCode::Char(' '))]);
    }

    #[test]
    fn function_keys() {
        assert_eq!(parse("<f1>").unwrap(), vec![key(KeyCode::F(1))]);
        assert_eq!(parse("<f12>").unwrap(), vec![key(KeyCode::F(12))]);
    }

    #[test]
    fn unknown_key_errors() {
        assert!(parse("<bogus>").is_err());
    }

    #[test]
    fn unclosed_angle_errors() {
        assert!(parse("<esc").is_err());
    }

    #[test]
    fn empty_sequence_errors() {
        assert!(parse("").is_err());
    }
}
