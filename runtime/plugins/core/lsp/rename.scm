;;; core:lsp/rename.scm — textDocument/rename. See docs/features.md.

(require "lib.scm")

(define-command! "lsp-rename" "Rename the symbol under the cursor."
  (lambda ()
    (lsp/guard-capability "renameProvider"
      (lambda ()
        (let ((bid (current-buffer)))
          (prompt! "Rename: "
            (lambda (new-name)
              (when new-name
                (lsp-request #f "textDocument/rename"
                  (hash-insert (lsp-position-params bid) "newName" new-name)
                  (lambda (err res)
                    (cond
                      (err (lsp/report-error "rename" err))
                      ((void? res) (log! 'info "Nothing to rename"))
                      (else (apply-workspace-edit! res)))))))
            #:prefill (symbol-under-cursor bid)))))))
