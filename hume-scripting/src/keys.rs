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
//! "esc"       → [Escape]
//! "g h"       → [Char('g'), Char('h')]
//! "m d"       → [Char('m'), Char('d')]
//! ```

use termina::event::{KeyCode, KeyEvent, Modifiers};

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
    let mut modifiers = Modifiers::NONE;
    let mut rest_lower = lower.as_str();

    // Strip all modifier prefixes — order doesn't matter.
    loop {
        if let Some(tail) = rest_lower.strip_prefix("ctrl-") {
            modifiers |= Modifiers::CONTROL;
            rest_lower = tail;
        } else if let Some(tail) = rest_lower.strip_prefix("shift-") {
            modifiers |= Modifiers::SHIFT;
            rest_lower = tail;
        } else if let Some(tail) = rest_lower.strip_prefix("alt-") {
            modifiers |= Modifiers::ALT;
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
    // Terminal input reports Shift+Tab as `BackTab`, not `Tab | SHIFT`.
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
        "esc" | "escape" => return Ok(KeyCode::Escape),
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
        return Ok(KeyCode::Function(n));
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
/// "wbc<esc>"    → [Char('w'), Char('b'), Char('c'), Escape]
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
            keys.push(KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE));
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
        return Ok(KeyEvent::new(KeyCode::Char('<'), Modifiers::NONE));
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

/// Terminal input reports Shift+Tab as `BackTab`, not `Tab | SHIFT`.
fn normalise_shift_tab(code: KeyCode, mods: Modifiers) -> (KeyCode, Modifiers) {
    if code == KeyCode::Tab && mods.contains(Modifiers::SHIFT) {
        (KeyCode::BackTab, mods)
    } else {
        (code, mods)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
