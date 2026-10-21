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
mod tests {
    use super::*;

    fn words(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_splits_on_any_whitespace() {
        assert_eq!(
            parse_word_list("  sh \t -c ").unwrap(),
            words(&["sh", "-c"])
        );
    }

    #[test]
    fn parse_groups_a_quoted_word_with_spaces() {
        assert_eq!(
            parse_word_list(r#""C:\Program Files\Git\bin\bash.exe" -c"#).unwrap(),
            words(&[r"C:\Program Files\Git\bin\bash.exe", "-c"])
        );
    }

    #[test]
    fn parse_unescapes_a_quote_and_a_backslash_inside_quotes() {
        assert_eq!(
            parse_word_list(r#""a \"b\" \\c""#).unwrap(),
            words(&[r#"a "b" \c"#])
        );
    }

    #[test]
    fn parse_keeps_a_backslash_outside_quotes() {
        assert_eq!(
            parse_word_list(r"C:\bin\sh").unwrap(),
            words(&[r"C:\bin\sh"])
        );
    }

    #[test]
    fn parse_joins_a_quoted_segment_to_the_word_around_it() {
        assert_eq!(parse_word_list(r#"a"b c"d"#).unwrap(), words(&["ab cd"]));
    }

    #[test]
    fn parse_keeps_an_empty_quoted_word() {
        assert_eq!(parse_word_list(r#"x """#).unwrap(), words(&["x", ""]));
    }

    #[test]
    fn parse_refuses_no_words() {
        assert!(parse_word_list("").is_err());
        assert!(parse_word_list("   ").is_err());
    }

    #[test]
    fn parse_refuses_an_unterminated_quote() {
        assert!(parse_word_list(r#"sh "-c"#).is_err());
    }

    #[test]
    fn format_round_trips_through_parse() {
        let cases = [
            words(&["sh", "-c"]),
            words(&[r"C:\Program Files\sh.exe", "-c"]),
            words(&[r#"say "hi""#, r"back\slash", r"C:\bin"]),
            words(&["", "x"]),
            words(&["tab\there"]),
        ];
        for case in cases {
            let text = format_word_list(&case);
            assert_eq!(parse_word_list(&text).unwrap(), case, "via {text:?}");
        }
    }

    #[test]
    fn format_leaves_plain_words_bare() {
        assert_eq!(format_word_list(&words(&["bash", "-lc"])), "bash -lc");
    }
}
