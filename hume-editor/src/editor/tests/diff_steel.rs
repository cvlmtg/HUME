// End-to-end Steel coverage for `diff-lines` / `diff-buffer-lines` (Phase 2a,
// docs/GIT-DIFF.md).

use std::path::Path;

use super::*;
use hume_scripting::ScriptingHost;

/// Runs `body` as a Steel command; the command moves the cursor iff `body`'s
/// own assertion (embedded in the Scheme source) held. Local copy of the
/// identical helper in `lsp_introspect.rs` — no shared home for it yet.
fn run_probe(ed: &mut Editor, host: ScriptingHost, tmp: &Path, body: &str) -> bool {
    let mut host = host;
    let source = format!(
        r#"(define-command! "probe" "" (lambda () (when (begin {body}) (call! "move-right"))))"#
    );
    eval_with_real_host(ed, &mut host, &source, tmp);
    ed.scripting = Some(host);
    let before = state(ed);
    type_cmd(ed, ":probe");
    state(ed) != before
}

/// `diff-lines` returns 0-based hunk tuples, oldest side first.
///
/// Fail oracle: any change to field order, base, or list-vs-vector encoding
/// at the Steel boundary stops this probe from firing — it is the one test
/// that pins the *registered* Steel shape, not just the Rust struct.
#[test]
fn diff_lines_returns_zero_based_hunk_tuples() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (diff-lines "a\nb\nc\n" "a\nB\nc\n")
                   (list (list 1 1 1 1 (list "b") (list "B"))))"#,
    );
    assert!(fired, "diff-lines must return the expected hunk shape");
}

/// `diff-buffer-lines` diffs `ref-text` (old) against the live buffer (new)
/// — this pins the argument order: `old-lines` is the ref's line,
/// `new-lines` is the buffer's.
///
/// Fail oracle: swap ref/buffer inside `DiffHost::diff_buffer_lines` — the
/// hunk's old/new sides invert and the probe stops firing. Also stands in
/// for the doc's `diff-buffer-lines` ≡ `diff-lines` equivalence check —
/// since both route through the same `diff_bridge::line_hunks`, this single
/// assertion (same texts fed both ways) is structurally guaranteed rather
/// than needing a matrix.
#[test]
fn diff_buffer_lines_diffs_the_live_buffer_against_the_ref() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (diff-buffer-lines (current-buffer) "a\nB\nc\n")
                   (list (list 1 1 1 1 (list "B") (list "b"))))"#,
    );
    assert!(
        fired,
        "diff-buffer-lines must diff the ref against the buffer's live text"
    );
}
