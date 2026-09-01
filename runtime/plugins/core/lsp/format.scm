;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting. Format-
;;; on-save is NOT wired by default — v1 is manual `:lsp-fmt` only. To opt
;;; in, uncomment:
;; (register-hook! 'on-buffer-save (lambda (bid) (call! "lsp-fmt")))

(require "lib.scm")

(define (lsp/format-options)
  (hash "tabSize" (get-option "tab-width")
        "insertSpaces" (equal? (get-option "tab-style") "soft")))

(define-command! "lsp-fmt"
  ":lsp-fmt — format the buffer via LSP, or just the selected lines if every selection spans one or more complete, contiguous lines."
  (lambda ()
    (let* ((bid (current-buffer))
           ;; `lsp-selections-range-params` itself returns #f when the
           ;; selections aren't contiguous (a single LSP range can't skip the
           ;; untouched line between two disjoint linewise selections) — that
           ;; folds straight into the same whole-buffer fallback as a
           ;; non-linewise selection.
           (range-rp (and (selections-linewise? bid) (lsp-selections-range-params bid)))
           (cap (if range-rp "documentRangeFormattingProvider" "documentFormattingProvider")))
      (lsp/guard-capability cap
        (lambda ()
          (let* ((gen (buffer-generation bid))
                 (rp (or range-rp (lsp-primary-range-params bid)))
                 (params (if range-rp
                             (hash-insert rp "options" (lsp/format-options))
                             (hash "textDocument" (hash-ref rp "textDocument")
                                   "options" (lsp/format-options))))
                 (method (if range-rp "textDocument/rangeFormatting" "textDocument/formatting")))
            (lsp-request #f method params
              (lambda (err res)
                (cond
                  (err (lsp/report-error "lsp-fmt" err))
                  ((void? res) (log! 'info "Already formatted"))
                  ((null? res) (log! 'info "Already formatted"))
                  (else (apply-text-edits! bid (map lsp/text-edit->tuple res)
                                            #:expect-generation gen)))))))))))
