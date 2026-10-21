use std::path::PathBuf;

use super::*;

fn env(key: &str) -> Result<String, std::env::VarError> {
    match key {
        "HOME" => Ok("/home/me".to_owned()),
        "X" => Ok("x-value".to_owned()),
        _ => Err(std::env::VarError::NotPresent),
    }
}

fn home() -> Option<PathBuf> {
    Some(PathBuf::from("/home/me"))
}

#[test]
fn escaping_backslashes_every_special_character() {
    assert_eq!(escape("plain-name.txt"), "plain-name.txt");
    assert_eq!(escape("my notes.txt"), r"my\ notes.txt");
    assert_eq!(escape("a$b'c\"d"), r#"a\$b\'c\"d"#);
    assert_eq!(escape(r"x\y*z"), r"x\\y\*z");
}

#[test]
fn escaping_quotes_a_newline() {
    assert_eq!(escape("a\nb c"), "a'\n'b\\ c");
}

#[test]
fn the_value_of_an_escaped_name_is_the_name() {
    let name = "a b\t$c'd\"e\\f|g;h&i(j)k<l>m*n?o[p]q{r}s!t#u~v`w";
    assert_eq!(prefix_value_with(&escape(name), env, home), name);
    assert_eq!(prefix_value_with(&escape("~"), env, home), "~");
}

#[test]
fn a_prefix_value_expands_unescaped_tilde_and_variables_only() {
    for (word, expected) in [
        ("plain", "plain"),
        (r"my\ dir/", "my dir/"),
        ("~", "~"),
        ("~/x", "/home/me/x"),
        ("~user/x", "~user/x"),
        (r"\~/x", "~/x"),
        ("a~/x", "a~/x"),
        ("$HOME/x", "/home/me/x"),
        ("${X}/y", "x-value/y"),
        (r"\$HOME/x", "$HOME/x"),
        (r"$X\ y", "x-value y"),
        ("$NOPE/x", "$NOPE/x"),
        ("a$", "a$"),
        (r"trailing\", "trailing"),
    ] {
        assert_eq!(prefix_value_with(word, env, home), expected, "{word:?}");
    }
}

#[test]
fn a_tilde_with_no_home_stays_literal() {
    assert_eq!(prefix_value_with("~/x", env, || None), "~/x");
}

#[test]
fn a_whole_prefix_never_splits_an_escape_or_a_quote() {
    for (s, expected) in [
        ("", ""),
        ("abc", "abc"),
        (r"a\ b", r"a\ b"),
        (r"a\", "a"),
        (r"a\\", r"a\\"),
        (r"a\\\", r"a\\"),
        ("a'\n'b", "a'\n'b"),
        ("a'\n", "a"),
        ("a'", "a"),
    ] {
        assert_eq!(whole_prefix(s), expected, "{s:?}");
    }
}

#[test]
fn a_word_splits_at_its_last_slash() {
    assert_eq!(split_at_sep("a/b/c"), ("a/b/", "c"));
    assert_eq!(split_at_sep("/tmp/"), ("/tmp/", ""));
    assert_eq!(split_at_sep(r"a\b"), ("", r"a\b"));
}

/// Marks the cursor in a test line; `|` would read as an operator.
const CURSOR: char = '‸';

#[test]
fn the_word_at_the_cursor_and_whether_it_names_a_program() {
    for (line, expected, command_position) in [
        ("‸", "", true),
        ("gr‸", "gr", true),
        ("  gr‸", "gr", true),
        ("ls gr‸", "gr", false),
        ("ls ‸", "", false),
        ("ls | gr‸", "gr", true),
        ("ls | ‸", "", true),
        ("ls|gr‸", "gr", true),
        ("a || gr‸", "gr", true),
        ("a && gr‸", "gr", true),
        ("a & gr‸", "gr", true),
        ("a; gr‸", "gr", true),
        ("(gr‸", "gr", true),
        ("echo $(gr‸", "gr", true),
        ("sort > ou‸", "ou", false),
        ("cat <in‸", "in", false),
        (r"cat my\ di‸", r"my\ di", false),
        (r"cat a\|b‸", r"a\|b", false),
        ("cat fi‸le other", "file", false),
        ("cat fi‸le|wc", "file", false),
        ("cat 5.48\u{202f}P‸M.png", "5.48\u{202f}PM.png", false),
        ("cat a\u{a0}b‸ c", "a\u{a0}b", false),
        (r"cat my\‸ dir", r"my\ dir", false),
        (r"cat my\‸ \ dir x", r"my\ \ dir", false),
        (r"cat a\\‸ b", r"a\\", false),
    ] {
        let cursor = line.find(CURSOR).expect("a cursor mark");
        let input = line.replacen(CURSOR, "", 1);
        let word = word_at(&input, cursor);
        assert_eq!(
            (&input[word.span], word.command_position),
            (expected, command_position),
            "{line:?}"
        );
    }
}

#[test]
fn blanks_are_space_tab_and_newline() {
    assert!([' ', '\t', '\n'].into_iter().all(is_blank));
    assert!(!['\u{202f}', '\u{a0}', '\r', 'a'].into_iter().any(is_blank));
}
