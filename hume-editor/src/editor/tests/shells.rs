//! `runtime/scheme/shells.scm`: each shell dialect's word rules, read and
//! written back. The `|` line's completion they drive is in `pipe.rs`.

use super::*;
use hume_scripting::ScriptingHost;
use steel::rvals::SteelVal;

/// Marks the cursor in a test line; `|` would read as an operator.
const CURSOR: char = '‸';

/// A scripting host that has evaluated `shells.scm`, as startup does.
fn shells_host() -> ScriptingHost {
    let source = std::fs::read_to_string(repo_runtime_dir().join("scheme/shells.scm")).unwrap();
    let mut host = ScriptingHost::new(&repo_runtime_dirs());
    host.eval_source(&source, &mut crate::testing::MockHost::new())
        .expect("shells.scm evaluates");
    host
}

/// `s` as a Steel string literal.
fn scheme_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn eval(host: &mut ScriptingHost, source: &str) -> SteelVal {
    host.eval_source_for_value(source, &mut crate::testing::MockHost::new())
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

fn eval_string(host: &mut ScriptingHost, source: &str) -> String {
    match eval(host, source) {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("{source}: expected a string, got {other:?}"),
    }
}

/// The word a dialect's `parse` returns: its text on the line and the
/// record's other fields.
#[derive(Debug, PartialEq)]
struct Word {
    text: String,
    command: bool,
    typed: String,
    dir: String,
    open: Option<char>,
    prefix: String,
}

impl Word {
    /// The value of the word up to the cursor.
    fn value(&self) -> String {
        format!("{}{}", self.dir, self.prefix)
    }
}

fn string_of(value: &SteelVal) -> String {
    match value {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("expected a string, got {other:?}"),
    }
}

/// The word `dialect`'s `parse` finds around the cursor marked in `line`,
/// at the end of `line` when it has no mark.
fn parse(host: &mut ScriptingHost, dialect: &str, line: &str) -> Word {
    let cursor = line
        .chars()
        .position(|c| c == CURSOR)
        .unwrap_or(line.chars().count());
    let input = line.replacen(CURSOR, "", 1);
    let SteelVal::ListV(fields) = eval(
        host,
        &format!(
            "(let ((w (shell/{dialect}-parse {} {cursor})))
               (list (shell-word-start w) (shell-word-end w) (shell-word-command? w)
                     (shell-word-typed w) (shell-word-dir w) (shell-word-open w)
                     (shell-word-prefix w)))",
            scheme_str(&input)
        ),
    ) else {
        panic!("expected a list");
    };
    let fields: Vec<SteelVal> = fields.iter().cloned().collect();
    let [
        SteelVal::IntV(start),
        SteelVal::IntV(end),
        SteelVal::BoolV(command),
        typed,
        dir,
        open,
        prefix,
    ] = &fields[..]
    else {
        panic!("parse answers a shell-word, got {fields:?}");
    };
    Word {
        text: input
            .chars()
            .skip(*start as usize)
            .take((end - start) as usize)
            .collect(),
        command: *command,
        typed: string_of(typed),
        dir: string_of(dir),
        open: match open {
            SteelVal::CharV(c) => Some(*c),
            SteelVal::BoolV(false) => None,
            other => panic!("open is a quote or #f, got {other:?}"),
        },
        prefix: string_of(prefix),
    }
}

/// What `dialect` writes for `name` after the word `typed`, the whole
/// line, and closes when `close`.
fn write(host: &mut ScriptingHost, dialect: &str, typed: &str, name: &str, close: bool) -> String {
    eval_string(
        host,
        &format!(
            "(shell/{dialect}-write (shell/{dialect}-parse {typed} {cursor}) {} {})",
            scheme_str(name),
            if close { "#t" } else { "#f" },
            typed = scheme_str(typed),
            cursor = typed.chars().count(),
        ),
    )
}

/// Each `(line, text, command)`: the word around the cursor in `line` and
/// whether it names a program.
fn assert_words(dialect: &str, cases: &[(&str, &str, bool)]) {
    let mut host = shells_host();
    for &(line, text, command) in cases {
        let word = parse(&mut host, dialect, line);
        assert_eq!(
            (word.text.as_str(), word.command),
            (text, command),
            "{line:?}"
        );
    }
}

/// Each `(word, value, open)`: what the shell reads for `word`, and the
/// quote open after its directory part.
fn assert_values(dialect: &str, cases: &[(String, String, Option<char>)]) {
    let mut host = shells_host();
    for (text, value, open) in cases {
        let word = parse(&mut host, dialect, text);
        assert_eq!(
            (word.value(), word.open),
            (value.clone(), *open),
            "{text:?}"
        );
    }
}

/// Each `(line, typed, dir, open, prefix)`: how the word around the cursor
/// splits into the directory kept as typed and the name being completed.
fn assert_splits(dialect: &str, cases: &[(&str, &str, String, Option<char>, &str)]) {
    let mut host = shells_host();
    for (line, typed, dir, open, prefix) in cases {
        let word = parse(&mut host, dialect, line);
        assert_eq!(
            (
                word.typed.as_str(),
                word.dir.as_str(),
                word.open,
                word.prefix.as_str()
            ),
            (*typed, dir.as_str(), *open, *prefix),
            "{line:?}"
        );
    }
}

/// Each `(typed, name, close, expected)`: what `dialect` writes for `name`
/// after the word `typed`.
fn assert_writes(dialect: &str, cases: &[(&str, &str, bool, String)]) {
    let mut host = shells_host();
    for (typed, name, close, expected) in cases {
        assert_eq!(
            &write(&mut host, dialect, typed, name, *close),
            expected,
            "{typed:?} {name:?} {close}"
        );
    }
}

/// A name with every character a shell reads specially.
const NASTY_NAME: &str = "a b\t$c'd\"e\\f|g;h&i(j)k<l>m*n?o[p]q{r}s!t#u~v`w";

// ── Choosing a dialect ───────────────────────────────────────────────────────

#[test]
fn a_dialect_is_chosen_by_the_shell_program_file() {
    let mut host = shells_host();
    for program in ["sh", "/bin/bash", "ZSH", r"C:\tools\Bash.EXE"] {
        assert!(
            matches!(
                eval(
                    &mut host,
                    &format!(
                        "(equal? (%shell-dialect-for {}) (%shell-dialect-for \"sh\"))",
                        scheme_str(program)
                    )
                ),
                SteelVal::BoolV(true)
            ),
            "{program}"
        );
    }
    assert!(matches!(
        eval(&mut host, "(%shell-dialect-for \"nu\")"),
        SteelVal::BoolV(false)
    ));
}

#[test]
fn a_dialect_registered_later_replaces_one_of_the_same_name() {
    let mut host = shells_host();
    eval(
        &mut host,
        r#"(register-shell-dialect! "posix" #:programs '("probe-sh")
             #:parse shell/posix-parse #:write shell/posix-write)"#,
    );
    assert!(matches!(
        eval(&mut host, "(%shell-dialect-for \"sh\")"),
        SteelVal::BoolV(false)
    ));
    assert!(!matches!(
        eval(&mut host, "(%shell-dialect-for \"probe-sh\")"),
        SteelVal::BoolV(false)
    ));
}

/// `name`'s value as `rest` read after `$NAME` expands, or the reference
/// itself when the environment has no `name`.
fn with_env(name: &str, reference: &str, rest: &str) -> String {
    match std::env::var(name) {
        Ok(value) => format!("{value}{rest}"),
        Err(_) => format!("{reference}{rest}"),
    }
}

fn home() -> String {
    hume_platform::dirs::home_dir()
        .expect("a home directory")
        .to_string_lossy()
        .into_owned()
}

/// `dialect` reads back each name it writes after each typed word as the
/// typed directory and the name, the finished word with no quote left open
/// (a blank after it ends it), the unfinished one as the same value.
fn reads_back_what_it_writes(dialect: &str, typed_dirs: &[&str], names: &[&str]) {
    let mut host = shells_host();
    for typed in typed_dirs {
        let dir = parse(&mut host, dialect, typed).dir;
        for name in names {
            let written = write(&mut host, dialect, typed, name, true);
            let read = parse(&mut host, dialect, &format!("{written}{CURSOR} x"));
            assert_eq!(
                (read.value(), read.text),
                (format!("{dir}{name}"), written.clone()),
                "{typed:?} {name:?} → {written:?}"
            );
            let partial = write(&mut host, dialect, typed, name, false);
            assert_eq!(
                parse(&mut host, dialect, &partial).value(),
                format!("{dir}{name}"),
                "{typed:?} {name:?} → {partial:?}"
            );
        }
    }
}

// ── POSIX sh ─────────────────────────────────────────────────────────────────

#[test]
fn posix_words_end_at_unquoted_blanks_and_operators() {
    assert_words(
        "posix",
        &[
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
            (r"probe\ to‸", r"probe\ to", true),
            ("cat \"my no‸", "\"my no", false),
            ("cat 'my no‸", "'my no", false),
            ("echo \"a|b‸\" c", "\"a|b\"", false),
            ("echo \"a|b c‸", "\"a|b c", false),
            ("cat \"a\\\"b c‸", "\"a\\\"b c", false),
            ("cat 'a\\'‸ b", "'a\\'", false),
        ],
    );
}

#[test]
fn posix_word_values_unescape_unquote_and_expand() {
    let home = home();
    let home_var = |rest: &str| with_env("HOME", "$HOME", rest);
    let none = None;
    let dq = Some('"');
    let sq = Some('\'');
    let s = str::to_owned;
    assert_values(
        "posix",
        &[
            (s("plain"), s("plain"), none),
            (s(r"my\ dir/"), s("my dir/"), none),
            (s("~"), s("~"), none),
            (s("~/x"), format!("{home}/x"), none),
            (s("~user/x"), s("~user/x"), none),
            (s(r"\~/x"), s("~/x"), none),
            (s("a~/x"), s("a~/x"), none),
            (s("$HOME/x"), home_var("/x"), none),
            (s("${HOME}/y"), home_var("/y"), none),
            (s(r"\$HOME/x"), s("$HOME/x"), none),
            (s("$HUME_NO_SUCH_VAR/x"), s("$HUME_NO_SUCH_VAR/x"), none),
            (s("${HUME_NO_SUCH_VAR"), s("${HUME_NO_SUCH_VAR"), none),
            (s("a$"), s("a$"), none),
            (s(r"trailing\"), s("trailing"), none),
            (s("'$HOME'"), s("$HOME"), sq),
            (s("\"$HOME/x"), home_var("/x"), dq),
            (s("\"my no"), s("my no"), dq),
            (s("'my no"), s("my no"), sq),
            (s("\"a\\\"b"), s("a\"b"), dq),
            (s("\"a\\qb"), s("a\\qb"), dq),
            (s("'a\\"), s("a\\"), sq),
            (s("a\"b c\"d"), s("ab cd"), none),
            (s("\"a b\""), s("a b"), dq),
        ],
    );
}

#[test]
fn posix_splits_a_word_after_its_last_directory() {
    let s = str::to_owned;
    assert_splits(
        "posix",
        &[
            ("cat \"my dir/\"", "\"my dir/", s("my dir/"), Some('"'), ""),
            ("cat \"my dir/", "\"my dir/", s("my dir/"), Some('"'), ""),
            ("cat dir/\"my", "dir/\"", s("dir/"), Some('"'), "my"),
            ("cat dir/'", "dir/'", s("dir/"), Some('\''), ""),
            ("cat ~/doc", "~/", format!("{}/", home()), None, "doc"),
            (r"cat \~/x", r"\~/", s("~/"), None, "x"),
            (
                "cat $HOME/x",
                "$HOME/",
                with_env("HOME", "$HOME", "/"),
                None,
                "x",
            ),
            (r"cat my\", "", s(""), None, "my"),
            (r"cat my\‸ dir", "", s(""), None, "my"),
            ("cat a\"b c\"/d", "a\"b c\"/", s("ab c/"), None, "d"),
            ("cat fi‸le", "", s(""), None, "fi"),
            ("cat a/b/‸c/d", "a/b/", s("a/b/"), None, ""),
        ],
    );
}

#[test]
fn posix_writes_a_name_in_the_quote_the_typed_word_left_open() {
    let s = str::to_owned;
    assert_writes(
        "posix",
        &[
            ("", "plain-name.txt", true, s("plain-name.txt")),
            ("", "my notes.txt", true, s(r"my\ notes.txt")),
            ("", "a$b'c\"d", true, s(r#"a\$b\'c\"d"#)),
            ("", r"x\y*z", true, s(r"x\\y\*z")),
            ("", "a\nb c", true, s("a'\n'b\\ c")),
            ("", "~", true, s(r"\~")),
            ("a\\ b/", "c d/", true, s("a\\ b/c\\ d/")),
            ("\"", "my notes.txt", true, s("\"my notes.txt\"")),
            ("\"", "my dir/", true, s("\"my dir/\"")),
            ("\"a b/", "x$y`z", true, s("\"a b/x\\$y\\`z\"")),
            ("'", "it's", true, s(r"'it'\''s'")),
            ("'", "a $b", true, s("'a $b'")),
            ("\"", "my not", false, s("\"my not")),
            ("", "my d", false, s(r"my\ d")),
        ],
    );
}

#[test]
fn posix_reads_back_what_it_writes() {
    reads_back_what_it_writes(
        "posix",
        &["", "\"", "'", "\"a b/", "a\\ b/"],
        &[NASTY_NAME, "~", "plain"],
    );
}

#[cfg(unix)]
#[test]
fn sh_reads_a_written_name_as_the_name() {
    let _path = path_reader();
    let mut host = shells_host();
    for typed in ["", "\"", "'"] {
        let written = write(&mut host, "posix", typed, NASTY_NAME, true);
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("printf %s {written}")])
            .output()
            .expect("sh runs");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            NASTY_NAME,
            "{typed:?} → {written:?}"
        );
    }
}

// ── fish ─────────────────────────────────────────────────────────────────────

#[test]
fn fish_words_name_a_program_after_and_or_not() {
    assert_words(
        "fish",
        &[
            ("gr‸", "gr", true),
            ("ls gr‸", "gr", false),
            ("ls | gr‸", "gr", true),
            ("and gr‸", "gr", true),
            ("not gr‸", "gr", true),
            ("ls; or gr‸", "gr", true),
            ("echo and gr‸", "gr", false),
            ("ls and‸", "and", false),
            (r"cat my\ di‸", r"my\ di", false),
            (r"cat 'a\'b c‸", r"'a\'b c", false),
            ("cat \"my no‸", "\"my no", false),
        ],
    );
}

#[test]
fn fish_word_values_follow_its_escapes() {
    let none = None;
    let dq = Some('"');
    let sq = Some('\'');
    let s = str::to_owned;
    assert_values(
        "fish",
        &[
            (s(r"my\ dir/"), s("my dir/"), none),
            (s("~/x"), format!("{}/x", home()), none),
            (s(r"\~/x"), s("~/x"), none),
            (s("$HOME/x"), with_env("HOME", "$HOME", "/x"), none),
            (s("${HOME}"), s("${HOME}"), none),
            (s(r"\q"), s("q"), none),
            (s(r"a\nb"), s("a\nb"), none),
            (s(r"'a\'b"), s("a'b"), sq),
            (s(r"'a\qb"), s(r"a\qb"), sq),
            (s(r#""a\"b"#), s("a\"b"), dq),
            (s(r#""a\qb"#), s(r"a\qb"), dq),
            (s(r#""a\$b"#), s("a$b"), dq),
            (s("'$HOME'"), s("$HOME"), sq),
        ],
    );
}

#[test]
fn fish_writes_a_name_in_the_quote_the_typed_word_left_open() {
    let s = str::to_owned;
    assert_writes(
        "fish",
        &[
            ("", "plain-name.txt", true, s("plain-name.txt")),
            ("", "my notes.txt", true, s(r"my\ notes.txt")),
            ("", "a{b,c}", true, s(r"a\{b,c\}")),
            ("", "a\nb", true, s(r"a\nb")),
            ("'", "it's", true, s(r"'it\'s'")),
            ("\"", "a$b", true, s(r#""a\$b""#)),
            ("\"", "my dir/", true, s("\"my dir/\"")),
            ("'", "my d", false, s("'my d")),
        ],
    );
}

#[test]
fn fish_reads_back_what_it_writes() {
    reads_back_what_it_writes(
        "fish",
        &["", "\"", "'", r"a\ b/"],
        &[NASTY_NAME, "~", "a\nb", "plain"],
    );
}

// ── PowerShell ───────────────────────────────────────────────────────────────

#[test]
fn pwsh_words_end_at_its_operators_outside_quotes() {
    assert_words(
        "pwsh",
        &[
            ("gr‸", "gr", true),
            ("ls | gr‸", "gr", true),
            ("ls; gr‸", "gr", true),
            ("{ gr‸", "gr", true),
            ("cat a,b‸", "b", false),
            ("cat a` b‸", "a` b", false),
            ("cat 'my no‸", "'my no", false),
            ("cat 'it''s a‸", "'it''s a", false),
        ],
    );
}

#[test]
fn pwsh_word_values_follow_its_escapes() {
    let _path = path_reader();
    let none = None;
    let dq = Some('"');
    let sq = Some('\'');
    let s = str::to_owned;
    assert_values(
        "pwsh",
        &[
            (s("a` b"), s("a b"), none),
            (s("'it''s"), s("it's"), sq),
            (s("'a`b"), s("a`b"), sq),
            (s("\"a`\"b"), s("a\"b"), dq),
            (s("\"a\"\"b"), s("a\"b"), dq),
            (s("~/x"), format!("{}/x", home()), none),
            (s("$HOME/x"), format!("{}/x", home()), none),
            (s("'$HOME'"), s("$HOME"), sq),
            (s("$env:PATH"), with_env("PATH", "$env:PATH", ""), none),
            (
                s("${env:HUME_NO_SUCH_VAR}"),
                s("${env:HUME_NO_SUCH_VAR}"),
                none,
            ),
            (s("$x"), s("$x"), none),
        ],
    );
}

#[test]
fn pwsh_reads_back_a_word_it_left_open() {
    assert_splits(
        "pwsh",
        &[(
            "cat 'dir/a b",
            "'dir/",
            "dir/".to_owned(),
            Some('\''),
            "a b",
        )],
    );
}

#[test]
fn pwsh_writes_a_path_that_needs_quoting_in_single_quotes() {
    let separator = if cfg!(windows) { "\\" } else { "/" };
    let s = str::to_owned;
    assert_writes(
        "pwsh",
        &[
            ("", "plain.txt", true, s("plain.txt")),
            ("", "my notes.txt", true, s("'my notes.txt'")),
            ("", "it's here", true, s("'it''s here'")),
            ("dir/", "a b", true, s("'dir/a b'")),
            ("", "my dir/", true, format!("'my dir{separator}'")),
            ("'", "a b", true, s("'a b'")),
            ("\"", "a$b", true, s("\"a`$b\"")),
            ("dir/", "a b", false, s("'dir/a b")),
        ],
    );
}

#[test]
fn pwsh_reads_back_what_it_writes() {
    reads_back_what_it_writes("pwsh", &["", "\"", "'", "a` b/"], &[NASTY_NAME, "plain"]);
}

#[cfg(windows)]
#[test]
fn powershell_reads_a_written_name_as_the_name() {
    let _path = path_reader();
    let mut host = shells_host();
    for typed in ["", "\"", "'"] {
        let written = write(&mut host, "pwsh", typed, NASTY_NAME, true);
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("Write-Output {written}")])
            .output()
            .expect("powershell runs");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end_matches(['\r', '\n']),
            NASTY_NAME,
            "{typed:?} → {written:?}"
        );
    }
}

// ── cmd ──────────────────────────────────────────────────────────────────────

#[test]
fn cmd_words_end_at_its_operators_outside_quotes() {
    assert_words(
        "cmd",
        &[
            ("gr‸", "gr", true),
            ("dir & gr‸", "gr", true),
            ("a | gr‸", "gr", true),
            ("type gr‸", "gr", false),
            ("type a^ b‸", "a^ b", false),
            ("type \"a b‸", "\"a b", false),
            ("type \"a&b‸", "\"a&b", false),
        ],
    );
}

#[test]
fn cmd_word_values_follow_its_escapes() {
    let _path = path_reader();
    let none = None;
    let dq = Some('"');
    let s = str::to_owned;
    assert_values(
        "cmd",
        &[
            (s("a^&b"), s("a&b"), none),
            (s("\"a b"), s("a b"), dq),
            (s("\"a^b"), s("a^b"), dq),
            (s("%PATH%"), with_env("PATH", "%PATH%", ""), none),
            (s("%HUME_NO_SUCH_VAR%x"), s("%HUME_NO_SUCH_VAR%x"), none),
        ],
    );
}

/// A backslash is a separator in `cmd`, never an escape, so a quoted
/// directory ending in one still closes its quote.
#[test]
fn cmd_splits_a_quoted_directory_ending_in_a_backslash() {
    let s = str::to_owned;
    assert_splits(
        "cmd",
        &[
            (
                r#"type "C:\Program Files\""#,
                r#""C:\Program Files\"#,
                s(r"C:\Program Files\"),
                Some('"'),
                "",
            ),
            (
                r"type C:\Windows\Sys",
                r"C:\Windows\",
                s(r"C:\Windows\"),
                None,
                "Sys",
            ),
        ],
    );
}

#[test]
fn cmd_quotes_a_path_that_needs_it_from_the_word_start() {
    let s = str::to_owned;
    assert_writes(
        "cmd",
        &[
            ("", "plain.txt", true, s("plain.txt")),
            ("", "my notes.txt", true, s("\"my notes.txt\"")),
            ("", "a&b", true, s("\"a&b\"")),
            (r"C:\", "Program Files/", true, s(r#""C:\Program Files\""#)),
            (r"C:\", "Windows/", true, s(r"C:\Windows\")),
            (r#""C:\"#, "x y.txt", true, s(r#""C:\x y.txt""#)),
            (r"C:\", "Program F", false, s(r#""C:\Program F"#)),
        ],
    );
}

#[test]
fn cmd_reads_back_what_it_writes() {
    reads_back_what_it_writes("cmd", &["", "\"", r"C:\"], &["a b&c^d(e)f,g;h=i", "plain"]);
}

// ── Dialects in use ──────────────────────────────────────────────────────────

/// `|` then `typed` and Tab, in an editor started over a runtime holding
/// `shells.scm` and the core plugin `plugin` (loaded from `init.scm` when
/// given), working in `dir` with `shell` as its shell.
fn tab_on_pipe_line(
    dir: &std::path::Path,
    shell: &str,
    plugin: Option<&str>,
    typed: &str,
) -> Editor {
    let runtime = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(runtime.path().join("scheme")).unwrap();
    std::fs::copy(
        repo_runtime_dir().join("scheme/shells.scm"),
        runtime.path().join("scheme/shells.scm"),
    )
    .unwrap();
    let mut init = String::new();
    if let Some(source) = plugin {
        let plugin_dir = runtime.path().join("plugins/core/shell-probe");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.scm"), source).unwrap();
        init.push_str("(load-plugin! \"core:shell-probe\")\n");
    }
    init.push_str(&format!(
        "(set-option! \"shell\" '({} \"-c\"))",
        scheme_str(shell)
    ));
    let mut ed = editor_from("-[abc]>\n");
    load_with_init_in_runtime(&mut ed, runtime.path(), data.path(), &init);
    ed.state.cwd = dir.to_path_buf();
    ed.handle_key(key('|'));
    for ch in typed.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());
    ed.settle();
    ed
}

#[test]
fn the_shell_option_picks_the_dialect_that_completes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let ed = tab_on_pipe_line(dir.path(), "/usr/bin/fish", None, "cat 'my n");
    assert_eq!(minibuf_input(&ed), "cat 'my notes.txt'");
    let ed = tab_on_pipe_line(dir.path(), "pwsh", None, "cat my");
    assert_eq!(minibuf_input(&ed), "cat 'my notes.txt'");
}

#[test]
fn a_plugin_registers_a_dialect_for_its_shell() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let plugin = r#"(register-shell-dialect! "probe" #:programs '("probe-shell")
                     #:parse shell/posix-parse
                     #:write (lambda (word name close?)
                               (string-append (shell-word-typed word) "probe-" name)))"#;
    let ed = tab_on_pipe_line(dir.path(), "probe-shell", Some(plugin), "cat my");
    assert_eq!(minibuf_input(&ed), "cat probe-my notes.txt");
}

#[test]
fn a_dialect_whose_parse_returns_no_word_is_reported_by_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let plugin = r#"(register-shell-dialect! "probe" #:programs '("probe-shell")
                     #:parse (lambda (input cursor) '())
                     #:write shell/posix-write)"#;
    let ed = tab_on_pipe_line(dir.path(), "probe-shell", Some(plugin), "cat my");
    assert_eq!(minibuf_input(&ed), "cat my");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("shell dialect probe")),
        "expected an error naming the dialect"
    );
}
