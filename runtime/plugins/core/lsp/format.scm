;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting /
;;; rangesFormatting. See docs/features.md, including the format-on-save
;;; opt-in snippet.

(require "lib.scm")

(define (lsp/format-options)
  (hash "tabSize" (get-option "tab-width")
        "insertSpaces" (equal? (get-option "tab-style") "soft")))

(define (lsp/format-edits res)
  (if (or (void? res) (null? res))
      (list)
      (map lsp/text-edit->tuple res)))

(define (lsp/format-apply! bid gen edits)
  (if (null? edits)
      (log! 'info "Already formatted")
      (apply-text-edits! bid edits #:expect-generation gen)))

(define (lsp/format-callback bid gen)
  (lambda (err res)
    (if err
        (lsp/report-error "lsp-fmt" err)
        (lsp/format-apply! bid gen (lsp/format-edits res)))))

(define (lsp/format-fan-out! bid gen td ranges)
  (let ((pending (box (length ranges)))
        (edits (box (list)))
        (aborted (box #f))
        (opts (lsp/format-options)))
    (for-each
      (lambda (range)
        (lsp-request #f "textDocument/rangeFormatting"
          (hash "textDocument" td "range" range "options" opts)
          (lambda (err res)
            (unless (unbox aborted)
              (if err
                  (begin
                    (set-box! aborted #t)
                    (lsp/report-error "lsp-fmt" err))
                  (begin
                    (set-box! edits (append (unbox edits) (lsp/format-edits res)))
                    (set-box! pending (- (unbox pending) 1))
                    (when (= (unbox pending) 0)
                      (lsp/format-apply! bid gen (unbox edits)))))))))
      ranges)))

(define (lsp/format-linewise! bid gen td ranges)
  (lsp/guard-capability "documentRangeFormattingProvider"
    (lambda ()
      (let ((n (length ranges))
            (cap (get-option "lsp.format-max-ranges")))
        (cond
          ((and (> n 1) (lsp/cap-flag? "documentRangeFormattingProvider" "rangesSupport"))
           (lsp-request #f "textDocument/rangesFormatting"
             (hash "textDocument" td "ranges" ranges "options" (lsp/format-options))
             (lsp/format-callback bid gen)))
          ((> n cap)
           (log! 'info
                 (string-append (number->string n)
                                 " ranges exceeds lsp.format-max-ranges ("
                                 (number->string cap)
                                 ") — nothing formatted")))
          (else (lsp/format-fan-out! bid gen td ranges)))))))

;;; Shared body behind `lsp-fmt` and `:format-source` — see docs/features.md.
(define (lsp/format-source!)
  (let* ((bid (current-buffer))
         (rp (lsp-linewise-ranges-params bid)))
    (if (not rp)
        (log! 'info (if (lsp-server-for-buffer bid)
                         "buffer has no path — nothing to format"
                         "no LSP server attached to this buffer"))
        (let* ((td (hash-ref rp "textDocument"))
               (ranges (hash-ref rp "ranges"))
               (gen (buffer-generation bid)))
          (cond
            ((selections-linewise? bid) (lsp/format-linewise! bid gen td ranges))
            ((selections-charwise? bid)
             (lsp/guard-capability "documentFormattingProvider"
               (lambda ()
                 (lsp-request #f "textDocument/formatting"
                   (hash "textDocument" td "options" (lsp/format-options))
                   (lsp/format-callback bid gen)))))
            (else (log! 'info "mixed whole-line and partial selections — nothing formatted")))))))

(define-command! "lsp-fmt"
  "Format the buffer via LSP — bind this to a key, or call it from a hook (e.g. `on-buffer-save`)."
  (lambda () (lsp/format-source!)))

(define-typed-command! "format-source"
  ":format-source — format the buffer via LSP from the command line."
  (lambda () (lsp/format-source!)))
