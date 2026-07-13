;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting.
;;;
;;; Format-on-save is NOT wired by default — v1 is manual `:lsp-fmt` only. To
;;; opt in, uncomment:
;; (register-hook! 'on-buffer-save (lambda (bid) (call! "lsp-fmt")))

(require "lib.scm")

(define (lsp/format-options)
  (hash "tabSize" (get-option "tab-width")
        "insertSpaces" (equal? (get-option "tab-style") "soft")))

(define-command! "lsp-fmt"
  ":lsp-fmt — format the buffer via LSP, or just the selected lines if the \
selection spans one or more complete lines."
  (lambda ()
    (let* ((bid (current-buffer))
           (range? (selection-spans-full-line? bid))
           (cap (if range? "documentRangeFormattingProvider" "documentFormattingProvider")))
      (lsp/guard-capability cap
        (lambda ()
          (let* ((gen (buffer-generation bid))
                 (rp (lsp-range-params bid))
                 (params (if range?
                             (hash-insert rp "options" (lsp/format-options))
                             (hash "textDocument" (hash-ref rp "textDocument")
                                   "options" (lsp/format-options))))
                 (method (if range? "textDocument/rangeFormatting" "textDocument/formatting")))
            (lsp-request #f method params
              (lambda (err res)
                (cond
                  (err (lsp/report-error "lsp-fmt" err))
                  ((void? res) (log! 'info "Already formatted"))
                  ((null? res) (log! 'info "Already formatted"))
                  (else (apply-text-edits! bid (map lsp/text-edit->tuple res)
                                            #:expect-generation gen)))))))))))
