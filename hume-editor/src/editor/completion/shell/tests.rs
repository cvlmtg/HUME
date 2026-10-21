use std::path::Path;

use super::super::testing::*;
use super::*;
use crate::editor::buffer::store::BufferStore;
use crate::editor::registry::CommandRegistry;

/// A search path a test asserts is never looked up.
fn unread_path() -> Option<OsString> {
    panic!("completing a path must not read PATH")
}

/// Completes `input` with the cursor at its end, in `cwd`, reading
/// `search_path` for a program word: the span and every item's
/// `(label, insert text)`.
fn complete_at_end(
    input: &str,
    cwd: &Path,
    search_path: impl FnOnce() -> Option<OsString>,
) -> (Range<usize>, Vec<(String, String)>) {
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let (span, items) =
        complete_shell_with(input, input.len(), &ctx(&reg, &store, cwd), search_path);
    let texts = items
        .iter()
        .map(|i| (i.label.to_string(), i.insert_text().to_owned()))
        .collect();
    (span, texts)
}

#[test]
fn a_path_argument_completes_escaped_over_its_word() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    std::fs::write(dir.path().join("other"), b"").unwrap();
    assert_eq!(
        complete_at_end("cat my", dir.path(), unread_path),
        (4..6, vec![("my notes.txt".into(), r"my\ notes.txt".into())])
    );
}

#[test]
fn an_escaped_directory_is_looked_up_unescaped_and_kept_as_typed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a b")).unwrap();
    std::fs::write(dir.path().join("a b/c.txt"), b"").unwrap();
    let input = r"cat a\ b/c";
    assert_eq!(
        complete_at_end(input, dir.path(), unread_path),
        (4..input.len(), vec![("c.txt".into(), r"a\ b/c.txt".into())])
    );
}

#[test]
fn an_escaped_space_before_the_cursor_stays_in_the_file_prefix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let input = r"cat my\ n";
    assert_eq!(
        complete_at_end(input, dir.path(), unread_path),
        (
            4..input.len(),
            vec![("my notes.txt".into(), r"my\ notes.txt".into())]
        )
    );
}

#[test]
fn an_escaped_tilde_names_a_directory_called_tilde() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("~")).unwrap();
    std::fs::write(dir.path().join("~/inside-tilde"), b"").unwrap();
    let input = r"cat \~/";
    assert_eq!(
        complete_at_end(input, dir.path(), unread_path),
        (
            4..input.len(),
            vec![("inside-tilde".into(), r"\~/inside-tilde".into())]
        )
    );
}

#[test]
fn a_program_word_with_an_escaped_space_reads_the_search_path() {
    let dir = tempfile::tempdir().unwrap();
    let read = std::cell::Cell::new(false);
    complete_at_end(r"probe\ to", dir.path(), || {
        read.set(true);
        None
    });
    assert!(read.get(), "a backslash is an escape, not a separator");
}

#[test]
fn a_program_word_with_a_separator_completes_as_a_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("run.sh"), b"").unwrap();
    let input = "ls | ./ru";
    assert_eq!(
        complete_at_end(input, dir.path(), unread_path),
        (5..input.len(), vec![("run.sh".into(), "./run.sh".into())])
    );
}

#[test]
fn a_program_word_with_no_search_path_completes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("grep-me"), b"").unwrap();
    assert_eq!(
        complete_at_end("gr", dir.path(), || None),
        (0..2, vec![]),
        "a file in the working directory is no program"
    );
}

#[cfg(unix)]
mod unix;
