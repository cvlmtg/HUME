;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting /
;;; rangesFormatting. Format-on-save is NOT wired by default — v1 is manual
;;; `:lsp-fmt` only. To opt in, uncomment:
;; (register-hook! 'on-buffer-save (lambda (bid) (call! "lsp-fmt")))

(require "lib.scm")

(define (lsp/format-options)
  (hash "tabSize" (get-option "tab-width")
        "insertSpaces" (equal? (get-option "tab-style") "soft")))

(define (lsp/ranges-support?)
  (equal? (lsp/cap-field (lsp-capabilities #f) "documentRangeFormattingProvider"
                          "rangesSupport" #f)
          #t))

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

;;; `aborted` only suppresses duplicate error lines when two or more ranges
;;; fail — the no-partial-format guarantee comes from `pending` never
;;; reaching zero after an error, not from this box.
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
      (let ((n (length ranges)))
        (cond
          ((and (> n 1) (lsp/ranges-support?))
           (lsp-request #f "textDocument/rangesFormatting"
             (hash "textDocument" td "ranges" ranges "options" (lsp/format-options))
             (lsp/format-callback bid gen)))
          ((> n (get-option "lsp.format-max-ranges"))
           (log! 'warn
                 (string-append (number->string n)
                                 " ranges exceeds lsp.format-max-ranges ("
                                 (number->string (get-option "lsp.format-max-ranges"))
                                 ") — nothing formatted")))
          (else (lsp/format-fan-out! bid gen td ranges)))))))

(define-command! "lsp-fmt"
  ":lsp-fmt — format the buffer via LSP: the selected lines when every selection spans one or more complete lines, the whole buffer otherwise, or nothing (with a warning) for a mix of the two."
  (lambda ()
    (let* ((bid (current-buffer))
           (rp (lsp-linewise-ranges-params bid)))
      (if (not rp)
          ;; No path or no attached server — `lsp-server-for-buffer`
          ;; distinguishes the two, since a capability guard can't: without
          ;; a server there's no `lsp-capabilities` to check in the first
          ;; place.
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
              (else (log! 'warn "mixed whole-line and partial selections — nothing formatted"))))))))
