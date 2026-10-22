//! The text form of a word-list setting (`shell`), as `:set` takes it and
//! as the setting reads back as text.
//!
//! Words are separated by whitespace. A `"…"` segment is part of the word
//! around it and may hold whitespace; inside one, `\"` and `\\` stand for
//! `"` and `\`. A backslash anywhere else is an ordinary character, so a
//! Windows path needs no escaping unless it holds a space.

use std::borrow::Cow;

/// The words `text` spells. `Err` for no words or an unterminated quote.
pub(super) fn parse_word_list(text: &str) -> Result<Vec<String>, String> {
    const UNTERMINATED: &str = "unterminated quote";
    let mut words = Vec::new();
    // `Some` once a word has started, so `""` is an empty word, not none.
    let mut word: Option<String> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '\\')) => word.push(escaped),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => return Err(UNTERMINATED.to_string()),
                        },
                        Some(other) => word.push(other),
                        None => return Err(UNTERMINATED.to_string()),
                    }
                }
            }
            c if c.is_whitespace() => words.extend(word.take()),
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    if words.is_empty() {
        return Err("needs at least one word".to_string());
    }
    Ok(words)
}

/// The text [`parse_word_list`] reads back as `words`. A word is quoted
/// only when it is empty or holds whitespace or a `"`.
pub(super) fn format_word_list(words: &[String]) -> String {
    words
        .iter()
        .map(|word| quote_word(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_word(word: &str) -> Cow<'_, str> {
    if !word.is_empty() && !word.contains(|c: char| c == '"' || c.is_whitespace()) {
        return Cow::Borrowed(word);
    }
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('"');
    for c in word.chars() {
        if matches!(c, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    Cow::Owned(quoted)
}

#[cfg(test)]
mod tests;
