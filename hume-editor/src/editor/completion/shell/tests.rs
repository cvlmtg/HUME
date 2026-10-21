use super::super::testing::*;
use super::*;
use crate::editor::buffer::store::BufferStore;
use crate::editor::registry::CommandRegistry;

/// Marks the cursor in a test line; `|` would read as an operator.
const CURSOR: char = '‸';

/// `line` without its cursor mark, and the mark's byte offset.
fn split_cursor(line: &str) -> (String, usize) {
    let cursor = line.find(CURSOR).expect("a cursor mark");
    (line.replacen(CURSOR, "", 1), cursor)
}

/// A search path a test asserts is never looked up.
fn unread_path() -> Option<OsString> {
    panic!("completing a path must not read PATH")
}

/// `(label, insert text)` of every item.
fn texts(items: &[CompletionItem]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|i| (i.label.to_string(), i.insert_text().to_owned()))
        .collect()
}

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
        let (input, cursor) = split_cursor(line);
        let word = shell_word_at(&input, cursor);
        assert_eq!(
            (&input[word.span], word.command_position),
            (expected, command_position),
            "{line:?}"
        );
    }
}

#[test]
fn a_path_argument_completes_escaped_over_its_word() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    std::fs::write(dir.path().join("other"), b"").unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let input = "cat my";
    let (span, items) = complete_shell_with(
        input,
        input.len(),
        &ctx(&reg, &store, dir.path()),
        unread_path,
    );
    assert_eq!(span, 4..6);
    assert_eq!(
        texts(&items),
        [("my notes.txt".to_owned(), r"my\ notes.txt".to_owned())]
    );
}

#[test]
fn an_escaped_directory_is_looked_up_unescaped_and_kept_as_typed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a b")).unwrap();
    std::fs::write(dir.path().join("a b/c.txt"), b"").unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let input = r"cat a\ b/c";
    let (span, items) = complete_shell_with(
        input,
        input.len(),
        &ctx(&reg, &store, dir.path()),
        unread_path,
    );
    assert_eq!(span, 4..input.len());
    assert_eq!(
        texts(&items),
        [("c.txt".to_owned(), r"a\ b/c.txt".to_owned())]
    );
}

#[test]
fn an_escaped_space_before_the_cursor_stays_in_the_file_prefix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let input = r"cat my\ n";
    let (span, items) = complete_shell_with(
        input,
        input.len(),
        &ctx(&reg, &store, dir.path()),
        unread_path,
    );
    assert_eq!(span, 4..input.len());
    assert_eq!(
        texts(&items),
        [("my notes.txt".to_owned(), r"my\ notes.txt".to_owned())]
    );
}

#[test]
fn an_escaped_tilde_names_a_directory_called_tilde() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("~")).unwrap();
    std::fs::write(dir.path().join("~/inside-tilde"), b"").unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let input = r"cat \~/";
    let (span, items) = complete_shell_with(
        input,
        input.len(),
        &ctx(&reg, &store, dir.path()),
        unread_path,
    );
    assert_eq!(span, 4..input.len());
    assert_eq!(
        texts(&items),
        [("inside-tilde".to_owned(), r"\~/inside-tilde".to_owned())]
    );
}

#[test]
fn a_program_word_with_an_escaped_space_reads_the_search_path() {
    let dir = tempfile::tempdir().unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let read = std::cell::Cell::new(false);
    let input = r"probe\ to";
    complete_shell_with(input, input.len(), &ctx(&reg, &store, dir.path()), || {
        read.set(true);
        None
    });
    assert!(read.get(), "a backslash is an escape, not a separator");
}

#[test]
fn a_program_word_with_a_separator_completes_as_a_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("run.sh"), b"").unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let input = "ls | ./ru";
    let (span, items) = complete_shell_with(
        input,
        input.len(),
        &ctx(&reg, &store, dir.path()),
        unread_path,
    );
    assert_eq!(span, 5..input.len());
    assert_eq!(
        texts(&items),
        [("run.sh".to_owned(), "./run.sh".to_owned())]
    );
}

#[test]
fn a_program_word_with_no_search_path_completes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("grep-me"), b"").unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let input = "gr";
    let (span, items) =
        complete_shell_with(input, input.len(), &ctx(&reg, &store, dir.path()), || None);
    assert_eq!(span, 0..2);
    assert!(
        items.is_empty(),
        "a file in the working directory is no program"
    );
}

#[cfg(unix)]
mod unix;
