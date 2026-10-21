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
