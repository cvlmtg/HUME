//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent: a program is found by its execute bit.

use std::os::unix::fs::PermissionsExt;

use super::*;

/// A search path of one directory holding the programs `names`.
fn programs(names: &[&str]) -> tempfile::TempDir {
    let bin = tempfile::tempdir().unwrap();
    for name in names {
        let path = bin.path().join(name);
        std::fs::write(&path, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

#[test]
fn a_program_word_completes_from_the_search_path() {
    let bin = programs(&["probe-tool", "probe tool", "other"]);
    let cwd = tempfile::tempdir().unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let input = "ls | pro";
    let (span, items) =
        complete_shell_with(input, input.len(), &ctx(&reg, &store, cwd.path()), || {
            Some(bin.path().as_os_str().to_owned())
        });
    assert_eq!(span, 5..input.len());
    assert_eq!(
        texts(&items),
        [
            ("probe tool".to_owned(), r"probe\ tool".to_owned()),
            ("probe-tool".to_owned(), "probe-tool".to_owned()),
        ]
    );
}
