//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;
use hume_platform::path::test_support::write_script;

/// A failed compile reports what the compiler wrote to its error output.
#[test]
fn compile_grammar_failure_carries_the_compilers_error_output() {
    let tmp = TempDir::new().unwrap();
    write_script(
        tmp.path(),
        "hume-grammar-build",
        "echo 'parser.c: no such grammar' >&2\nexit 1\n",
        0o755,
    );
    let (mut h, src, out) = failing_compile(&tmp);
    let mut ctx = h.ctx();
    let err = compile_grammar(&mut ctx, src, out).unwrap_err().to_string();
    assert!(
        err.contains("failed (exit code 1): parser.c: no such grammar"),
        "{err}"
    );
}
