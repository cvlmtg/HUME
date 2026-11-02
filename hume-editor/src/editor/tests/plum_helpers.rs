use super::*;

/// A raise from `plum/batch-run!`'s own bookkeeping after an item finished is
/// the command's error. The finished item stays counted as a success.
#[test]
fn batch_run_never_reports_a_finished_item_as_failed() {
    let data_tmp = tempfile::tempdir().unwrap();
    let helpers = repo_runtime_dir().join("plugins/core/plum/lib/helpers.scm");
    let mut ed = editor_from("-[x]>\n");
    ed.tui = crate::editor::tui::Tui::OnHeadless;
    load_plum_with(
        &mut ed,
        data_tmp.path(),
        &format!(
            r#"(require {})
               (define skip-raised #f)
               (define-typed-command! "test-batch" ""
                 (lambda ()
                   (plum/batch-run! "done" (list "a" "b") (lambda (name) #t)
                     #:skip (lambda (name failed)
                              (when (and (equal? name "b") (not skip-raised))
                                (set! skip-raised #t)
                                (error "skip broke"))
                              #f)))
                 #:inline-output #t)"#,
            steel_path(&helpers),
        ),
        None,
    );

    type_cmd(&mut ed, ":test-batch");

    let errors = error_log(&ed);
    assert!(
        !errors.iter().any(|e| e.starts_with("PLUM: a:")),
        "a finished, so it is not a failure: {errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("skip broke")),
        "the raise from #:skip fails the command: {errors:?}"
    );
}
