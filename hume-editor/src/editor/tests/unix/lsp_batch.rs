//! `stdlib/lsp-batch` over real `lsp-request!` calls: a batch under a batch
//! key replaces the previous batch of the same buffer.

use super::*;
use hume_lsp::test_util::RecordingLspBackend;

/// A `core:lsp` rig whose server offers completion and never answers, with
/// a `batch` command sending `n` completion requests as one batch under one
/// batch key, its callback logging `batch-done`.
fn unanswered_batch_rig(tmp: &std::path::Path) -> (LspRig, RealRuntimeDirs) {
    let (mut rig, guard) = core_lsp_rig(
        tmp,
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {"completionProvider": {}}}),
        |_: &mut RecordingLspBackend, _| {},
    );
    rig.eval(
        r#"(define batch-key (call! "stdlib/lsp-batch-key"))
           (define (send-batch bid n)
             (call! "stdlib/lsp-batch" bid
               (map (lambda (_)
                      (lambda (key done)
                        (lsp-request! bid "textDocument/completion" (hash) done #:supersede key)))
                    (range 0 n))
               (lambda (all) (log! 'trace "batch-done"))
               (hash 'supersede batch-key)))
           (define-typed-command! "batch-two" "" (lambda (bid) (send-batch bid 2)))
           (define-typed-command! "batch-one" "" (lambda (bid) (send-batch bid 1)))"#,
    );
    (rig, guard)
}

/// A shorter batch under the same key cancels every request of the one
/// before: the first by superseding it, the second as a leftover.
#[test]
fn a_shorter_batch_cancels_every_request_of_the_one_before() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _guard) = unanswered_batch_rig(tmp.path());
    let sid = rig.sid("rust-analyzer");

    type_cmd(&mut rig.ed, ":batch-two");
    settle_until_idle(&mut rig.ed);
    type_cmd(&mut rig.ed, ":batch-one");
    settle_until_idle(&mut rig.ed);

    assert_eq!(rig.sent(sid, "$/cancelRequest").len(), 2);
    assert_eq!(rig.ed.state.lsp.delivery_count_for_test(), 1);
    let log = rig.ed.state.message_log.format_for_display();
    assert!(!log.contains("batch-done"), "{log:?}");
}
