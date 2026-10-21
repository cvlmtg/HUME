//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent: a program is found by its execute bit.

use hume_platform::process::test_support::write_program;

use super::*;

#[test]
fn a_program_word_completes_from_the_search_path() {
    let bin = tempfile::tempdir().unwrap();
    for name in ["probe-tool", "probe tool", "other"] {
        write_program(bin.path(), name, 0o755);
    }
    let cwd = tempfile::tempdir().unwrap();
    let input = "ls | pro";
    let search_path = bin.path().as_os_str().to_owned();
    assert_eq!(
        complete_at_end(input, cwd.path(), || Some(search_path)),
        (
            5..input.len(),
            vec![
                ("probe tool".into(), r"probe\ tool".into()),
                ("probe-tool".into(), "probe-tool".into()),
            ]
        )
    );
}
