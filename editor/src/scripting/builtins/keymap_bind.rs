//! `(bind-key! mode key-sequence command-name)` builtin and key-string parser.
//!
//! Key strings use a space-separated token format:
//!
//! ```text
//! [modifier-]* key_name
//! ```
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

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use crate::editor::keymap::BindMode;
use crate::scripting::ledger::Owner;

type SteelResult = Result<SteelVal, SteelErr>;

// ── Key string parser ─────────────────────────────────────────────────────────

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
        return Err(format!("key token '{token}' has no key name after modifiers"));
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
        "space"                        => return Ok(KeyCode::Char(' ')),
        "tab"                          => return Ok(KeyCode::Tab),
        "enter" | "return" | "cr"      => return Ok(KeyCode::Enter),
        "esc"   | "escape"             => return Ok(KeyCode::Esc),
        "backspace" | "bs"             => return Ok(KeyCode::Backspace),
        "delete" | "del"               => return Ok(KeyCode::Delete),
        "insert" | "ins"               => return Ok(KeyCode::Insert),
        "home"                         => return Ok(KeyCode::Home),
        "end"                          => return Ok(KeyCode::End),
        "pageup"                       => return Ok(KeyCode::PageUp),
        "pagedown"                     => return Ok(KeyCode::PageDown),
        "up"                           => return Ok(KeyCode::Up),
        "down"                         => return Ok(KeyCode::Down),
        "left"                         => return Ok(KeyCode::Left),
        "right"                        => return Ok(KeyCode::Right),
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
        return Err(format!("key name is empty after modifiers"));
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

// ── Builtins ──────────────────────────────────────────────────────────────────

/// Parsed arguments shared by `bind_key` and `bind_wait_char`.
struct ParsedBindArgs {
    mode:       BindMode,
    /// `"<mode_lower> <key_str>"` — ledger key encoding both so that
    /// `"normal f"` and `"insert f"` are tracked independently.
    ledger_key: String,
    keys:       Vec<KeyEvent>,
    cmd_name:   String,
}

/// Validate and extract the three arguments common to `bind-key!` and
/// `bind-wait-char!`: `(mode key-sequence command-name)`.
fn parse_bind_args(args: &[SteelVal], fn_name: &str) -> Result<ParsedBindArgs, SteelErr> {
    if args.len() != 3 {
        steel::stop!(ArityMismatch =>
            "{fn_name} expects 3 args (mode key-sequence command-name), got {}", args.len());
    }
    let mode_str = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: first arg (mode) must be a string, got {:?}", args[0]),
    };
    let key_str = match &args[1] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: second arg (key-sequence) must be a string, got {:?}", args[1]),
    };
    let cmd_name = match &args[2] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: third arg (command-name) must be a string, got {:?}", args[2]),
    };
    let mode = match mode_str.to_ascii_lowercase().as_str() {
        "normal" => BindMode::Normal,
        "extend" => BindMode::Extend,
        "insert" => BindMode::Insert,
        _ => steel::stop!(Generic =>
            "{fn_name}: unknown mode '{}'; expected normal, extend, or insert", mode_str),
    };
    let keys = parse_key_sequence(&key_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    let ledger_key = format!("{} {}", mode_str.to_ascii_lowercase(), key_str);
    Ok(ParsedBindArgs { mode, ledger_key, keys, cmd_name })
}

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
    let ParsedBindArgs { mode, ledger_key, keys, cmd_name } = parse_bind_args(args, "bind-key!")?;
    super::with_ctx(|ctx| {
        let prior_value = ctx.keymap.lookup_command(mode, &keys).unwrap_or_default();
        let prior_owner = ctx.ledger_stack.owner_of(&ledger_key);
        let current_owner = ctx.plugin_stack.current_owner();
        ctx.keymap.bind_user(mode, &keys, Cow::Owned(cmd_name));
        if let Owner::Plugin(ref plugin_id) = current_owner {
            ctx.ledger_stack.record(plugin_id, ledger_key, prior_owner, prior_value);
        }
        Ok(SteelVal::Void)
    })
}

/// `(bind-wait-char! mode key-sequence command-name)`
///
/// Binds a key sequence to a WaitChar node so that after the user completes
/// the sequence, the next character is stored in `pending_char` and
/// `command-name` is dispatched.
///
/// Example: `(bind-wait-char! "normal" "m d" "helix-delete-surround")` makes
/// `m d <char>` dispatch `helix-delete-surround` with `(pending-char)` = char.
///
/// Records a ledger entry when called from a plugin body.
pub(crate) fn bind_wait_char(args: &[SteelVal]) -> SteelResult {
    let ParsedBindArgs { mode, ledger_key, keys, cmd_name } = parse_bind_args(args, "bind-wait-char!")?;
    super::with_ctx(|ctx| {
        let prior_value = ctx.keymap.lookup_command(mode, &keys).unwrap_or_default();
        let prior_owner = ctx.ledger_stack.owner_of(&ledger_key);
        let current_owner = ctx.plugin_stack.current_owner();
        ctx.keymap.bind_wait_char_user(mode, &keys, Cow::Owned(cmd_name));
        if let Owner::Plugin(ref plugin_id) = current_owner {
            ctx.ledger_stack.record(plugin_id, ledger_key, prior_owner, prior_value);
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
        assert_eq!(parse("up").unwrap(),    vec![key(KeyCode::Up)]);
        assert_eq!(parse("down").unwrap(),  vec![key(KeyCode::Down)]);
        assert_eq!(parse("left").unwrap(),  vec![key(KeyCode::Left)]);
        assert_eq!(parse("right").unwrap(), vec![key(KeyCode::Right)]);
    }

    #[test]
    fn function_keys() {
        assert_eq!(parse("f1").unwrap(),  vec![key(KeyCode::F(1))]);
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
        assert_eq!(
            parse("shift-tab").unwrap(),
            vec![shift(KeyCode::BackTab)],
        );
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
