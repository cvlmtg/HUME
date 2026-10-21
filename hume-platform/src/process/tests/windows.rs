//! Windows-only tests, gated once at the `mod windows;`
//! declaration in the parent.

use super::*;

/// npm-kind language servers are registered as a `.cmd` file, which
/// `spawn_piped` runs through `cmd.exe` from its suspended start in a job.
#[test]
fn spawn_piped_runs_a_batch_file_with_its_argument_intact() {
    use std::io::Read as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("echo-arg.cmd");
    std::fs::write(&script, "@echo %~1\r\n").expect("write script");
    let (child, stdin, mut stdout, _stderr) = spawn_piped(
        script.to_str().expect("UTF-8 temp path"),
        &["two words".to_string()],
        dir.path(),
        &[],
    )
    .expect("spawn batch file");
    drop(stdin);
    let mut out = String::new();
    stdout.read_to_string(&mut out).expect("read stdout");
    drop(child);
    assert_eq!(out, "two words\r\n");
}
