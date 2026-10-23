//! Windows-only tests, gated once at the `mod windows;`
//! declaration in the parent.

use super::*;

/// Writes a program `name` into `dir`, as `name.exe`.
pub(super) fn write_probe(dir: &Path, name: &str) {
    std::fs::write(dir.join(format!("{name}.exe")), b"").unwrap();
}

#[test]
fn an_exe_is_listed_by_its_stem_and_nothing_else_is_listed() {
    let bin = tempfile::tempdir().unwrap();
    for name in [
        "probe.exe",
        "probe-upper.EXE",
        "probe-script.cmd",
        "probe-batch.bat",
        "probe-notes.txt",
        "probe",
    ] {
        std::fs::write(bin.path().join(name), b"").unwrap();
    }
    assert_eq!(
        programs_in(&[bin.path()], "probe", &[".exe"]),
        ["probe", "probe-upper"]
    );
}

#[test]
fn each_given_extension_is_listed_by_its_stem() {
    let bin = tempfile::tempdir().unwrap();
    for name in [
        "probe.exe",
        "probe-script.cmd",
        "probe-batch.BAT",
        "probe-shell.ps1",
    ] {
        std::fs::write(bin.path().join(name), b"").unwrap();
    }
    assert_eq!(
        programs_in(&[bin.path()], "probe", &[".EXE", ".bat", ".ps1"]),
        ["probe", "probe-batch", "probe-shell"]
    );
}

#[test]
fn names_ignore_case_and_keep_the_first_spelling_on_the_path() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("Code.exe"), b"").unwrap();
    std::fs::write(second.path().join("CODE.exe"), b"").unwrap();
    std::fs::write(second.path().join("codex.exe"), b"").unwrap();
    assert_eq!(
        programs_in(&[first.path(), second.path()], "cod", &[".exe"]),
        ["Code", "codex"]
    );
}

#[test]
fn the_prefix_matches_against_the_stem() {
    let bin = tempfile::tempdir().unwrap();
    std::fs::write(bin.path().join("bash.exe"), b"").unwrap();
    assert_eq!(
        programs_in(&[bin.path()], "bash.e", &[".exe"]),
        Vec::<String>::new()
    );
}
