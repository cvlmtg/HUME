;;; core:lsp/rename.scm — textDocument/rename. See docs/features.md.

(require "lib.scm")

(define-command! "lsp-rename" "Rename the symbol under the cursor."
  (lambda (pane)
    (lsp/guard-capability pane "renameProvider"
      (lambda ()
        (prompt! pane "Rename: "
          (lambda (new-name)
            (when new-name
              (let ((gen (buffer-generation pane)))
                (lsp-request pane "textDocument/rename"
                  (hash-insert (lsp-position-params pane) "newName" new-name)
                  (lambda (err res)
                    (cond
                      (err (lsp/report-error "rename" err))
                      ((void? res) (log! 'info "Nothing to rename"))
                      (else (apply-workspace-edit! pane res #:expect-generation gen))))
                  #:allow-stale #t))))
          #:prefill (symbol-under-cursor pane))))))
