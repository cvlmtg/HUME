//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;
use crate::path::test_support::write_program;

/// Writes a program `name` into `dir`, one any user can run.
pub(super) fn write_probe(dir: &Path, name: &str) {
    write_program(dir, name, 0o755);
}

#[test]
fn files_with_an_execute_bit_are_programs() {
    let bin = tempfile::tempdir().unwrap();
    write_program(bin.path(), "probe-owner", 0o744);
    write_program(bin.path(), "probe-other", 0o601);
    write_program(bin.path(), "probe-plain", 0o644);
    assert_eq!(
        programs_in(&[bin.path()], "probe", &[".exe"]),
        ["probe-other", "probe-owner"]
    );
}

#[test]
fn programs_are_filtered_by_prefix() {
    let bin = tempfile::tempdir().unwrap();
    write_program(bin.path(), "alpha", 0o755);
    write_program(bin.path(), "beta", 0o755);
    assert_eq!(programs_in(&[bin.path()], "al", &[".exe"]), ["alpha"]);
}

#[test]
fn a_symlink_to_a_program_is_a_program() {
    let bin = tempfile::tempdir().unwrap();
    let real = write_program(bin.path(), "real", 0o755);
    let plain = write_program(bin.path(), "plain", 0o644);
    std::os::unix::fs::symlink(&real, bin.path().join("link-to-real")).unwrap();
    std::os::unix::fs::symlink(&plain, bin.path().join("link-to-plain")).unwrap();
    std::os::unix::fs::symlink(bin.path().join("gone"), bin.path().join("link-dangling")).unwrap();
    assert_eq!(
        programs_in(&[bin.path()], "link", &[".exe"]),
        ["link-to-real"]
    );
}

#[test]
fn a_name_on_two_directories_is_listed_once() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write_program(first.path(), "tool", 0o644);
    write_program(second.path(), "tool", 0o755);
    write_program(second.path(), "toolkit", 0o755);
    assert_eq!(
        programs_in(
            &[first.path(), second.path(), second.path()],
            "tool",
            &[".exe"]
        ),
        ["tool", "toolkit"]
    );
}
