//! POSIX `sh` word syntax: reading a typed word the way `sh` will, and
//! writing a name back so `sh` reads it unchanged. Quotes, backticks and
//! `$(…)` in a typed word are not read.

use std::borrow::Cow;
use std::path::PathBuf;

/// The path separator `sh` reads in a word, on every platform: a `\` is an
/// escape.
pub const PATH_SEP: char = '/';

/// The characters `sh` reads specially inside a word.
const SPECIAL: &[char] = &[
    ' ', '\t', '\n', '\\', '\'', '"', '`', '$', '&', ';', '|', '(', ')', '<', '>', '*', '?', '[',
    ']', '{', '}', '!', '#', '~',
];

/// Whether `c` ends a word: `sh` splits on space, tab and newline only.
pub fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n')
}

/// `word` split after its last [`PATH_SEP`] into `(dir, file)`, `dir`
/// keeping the separator; `("", word)` when it has none.
pub fn split_at_sep(word: &str) -> (&str, &str) {
    match word.rfind(PATH_SEP) {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word),
    }
}

/// `name` written so `sh` reads it as one word with this text: a backslash
/// before each character `sh` reads specially, and a newline in single
/// quotes, since a backslash before a newline joins two lines.
pub fn escape(name: &str) -> Cow<'_, str> {
    if !name.contains(SPECIAL) {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        match c {
            '\n' => out.push_str("'\n'"),
            c if SPECIAL.contains(&c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// The text `sh` reads `prefix`, the start of a word still being typed, as:
/// each `\x` pair is `x`, a leading `~` before `/` is the home directory,
/// and `$NAME`/`${NAME}` is the variable's value. A `~` with nothing after
/// it stays literal, since the word may go on to name `~user`. An escaped
/// `~` or `$` stays literal, and so does an unknown variable. A trailing
/// lone backslash is dropped.
pub fn prefix_value(prefix: &str) -> Cow<'_, str> {
    prefix_value_with(prefix, |k| std::env::var(k), crate::dirs::home_dir)
}

/// Testable core of [`prefix_value`].
fn prefix_value_with(
    prefix: &str,
    env_lookup: impl Fn(&str) -> Result<String, std::env::VarError>,
    home_fn: impl FnOnce() -> Option<PathBuf>,
) -> Cow<'_, str> {
    let tilde_tail = prefix
        .strip_prefix('~')
        .filter(|tail| tail.starts_with(PATH_SEP));
    if tilde_tail.is_none() && !prefix.contains(['\\', '$']) {
        return Cow::Borrowed(prefix);
    }
    let mut out = String::with_capacity(prefix.len());
    let mut rest = prefix;
    if let Some(tail) = tilde_tail {
        match home_fn() {
            Some(home) => out.push_str(&home.to_string_lossy()),
            None => out.push('~'),
        }
        rest = tail;
    }
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '\\' => {
                if let Some(escaped) = rest.chars().next() {
                    out.push(escaped);
                    rest = &rest[escaped.len_utf8()..];
                }
            }
            '$' => rest = &rest[crate::path::expand_dollar(rest, &env_lookup, &mut out)..],
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// The longest prefix of `escaped` (text [`escape`] wrote) that ends
/// neither between a backslash and the character it escapes nor inside a
/// quoted run.
pub fn whole_prefix(escaped: &str) -> &str {
    let mut chars = escaped.char_indices();
    let mut end = 0;
    while let Some((_, c)) = chars.next() {
        let closed = match c {
            '\\' => chars.next().is_some(),
            '\'' => chars.any(|(_, c)| c == '\''),
            _ => true,
        };
        if !closed {
            break;
        }
        end = chars.offset();
    }
    &escaped[..end]
}

#[cfg(test)]
mod tests;
