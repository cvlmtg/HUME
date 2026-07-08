;;; core:lsp/format.scm — F8: textDocument/formatting / rangeFormatting.
;;;
;;; Format-on-save is NOT wired by default — v1 is manual `:fmt` only. To
;;; opt in, uncomment:
;; (register-hook! 'on-buffer-save (lambda (bid) (call! "fmt")))

(require "lib.scm")

(define (lsp/format-options)
  (hash "tabSize" (get-option "tab-width")
        "insertSpaces" (equal? (get-option "tab-style") "soft")))

;;; A `TextEdit` hashmap `{range: {start, end}, newText}` -> the
;;; `((start-line start-col) (end-line end-col) text)` tuple shape
;;; `apply-text-edits!` expects.
(define (lsp/text-edit->tuple te)
  (let* ((range (hash-ref te "range"))
         (start (hash-ref range "start"))
         (end (hash-ref range "end")))
    (list (list (hash-ref start "line") (hash-ref start "character"))
          (list (hash-ref end "line") (hash-ref end "character"))
          (hash-ref te "newText"))))

(define-command! "fmt"
  ":fmt — format the buffer via LSP, or just the selected lines if the \
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
                  (err (lsp/report-error "fmt" err))
                  ((void? res) (log! 'info "Already formatted"))
                  ((null? res) (log! 'info "Already formatted"))
                  (else (apply-text-edits! bid (map lsp/text-edit->tuple res)
                                            #:expect-generation gen)))))))))))
