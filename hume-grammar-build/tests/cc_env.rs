#![cfg(unix)]

use std::fs;
use std::process::Command;

/// Extra words in `CC` reach the compiler: `cc` splits them off the program
/// name, as it does for `CC="ccache gcc"` or `CC="zig cc"`.
#[test]
fn every_word_of_cc_reaches_the_compiler() {
    let grammar = tempfile::tempdir().expect("grammar dir");
    let src = grammar.path().join("src");
    fs::create_dir(&src).expect("src dir");
    fs::write(
        src.join("parser.c"),
        "#ifndef HUME_CC_FLAG\n#error CC lost its flag\n#endif\n\
         int tree_sitter_probe(void) { return 0; }\n",
    )
    .expect("write parser");
    let out_dir = tempfile::tempdir().expect("out dir");
    let out = out_dir.path().join("probe.lib");

    let status = Command::new(env!("CARGO_BIN_EXE_hume-grammar-build"))
        .env("CC", "cc -DHUME_CC_FLAG")
        .arg("-o")
        .arg(&out)
        .arg(grammar.path())
        .status()
        .expect("run hume-grammar-build");

    assert!(status.success(), "build with CC carrying a flag: {status}");
    assert!(fs::metadata(&out).expect("library").len() > 0);
}

/// MSVC's `cl` writes its diagnostics to stdout, and `compile-grammar!`
/// reports only the builder's stderr, so a compiler's stdout must reach it.
#[test]
fn compiler_stdout_reaches_stderr() {
    use std::os::unix::fs::PermissionsExt;

    let grammar = tempfile::tempdir().expect("grammar dir");
    let src = grammar.path().join("src");
    fs::create_dir(&src).expect("src dir");
    fs::write(src.join("parser.c"), "").expect("write parser");
    let bin = tempfile::tempdir().expect("bin dir");
    let compiler = bin.path().join("fake-cc");
    // Answers `cc`'s family probe (`-E`) with nothing, and fails every
    // compile with a diagnostic on stdout.
    fs::write(
        &compiler,
        "#!/bin/sh\n[ \"$1\" = -E ] && exit 0\necho parser.c: fake-diagnostic\nexit 1\n",
    )
    .expect("write compiler");
    fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).expect("chmod");
    let out_dir = tempfile::tempdir().expect("out dir");

    let output = Command::new(env!("CARGO_BIN_EXE_hume-grammar-build"))
        .env("CC", &compiler)
        .arg("-o")
        .arg(out_dir.path().join("probe.lib"))
        .arg(grammar.path())
        .output()
        .expect("run hume-grammar-build");

    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("parser.c: fake-diagnostic"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
