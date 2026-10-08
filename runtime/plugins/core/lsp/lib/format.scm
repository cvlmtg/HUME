;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting /
;;; rangesFormatting. See docs/features.md.

(require "helpers.scm")

(define (lsp/format-options pane)
  (hash "tabSize" (get-buffer-option pane "tab-width")
        "insertSpaces" (equal? (get-buffer-option pane "tab-style") 'soft)))

(define (lsp/format-edits res)
  (if (void? res) (list) (json-list res)))

(define (lsp/format-apply! pane gen edits)
  (if (null? edits)
      (log! 'info "Already formatted")
      (apply-text-edits! pane edits #:expect-generation gen)))

(define (lsp/format-callback pane gen)
  (lambda (err res)
    (if err
        (lsp/report-error! "lsp-fmt" err)
        (lsp/format-apply! pane gen (lsp/format-edits res)))))

(define (lsp/format-fan-out! pane server gen td ranges)
  (let ((pending (box (length ranges)))
        (edits (box (list)))
        (aborted (box #f))
        (opts (lsp/format-options pane)))
    (for-each
      (lambda (range)
        (lsp-request! pane "textDocument/rangeFormatting"
          (hash "textDocument" td "range" range "options" opts)
          (lambda (err res)
            (unless (unbox aborted)
              (if err
                  (begin
                    (set-box! aborted #t)
                    (lsp/report-error! "lsp-fmt" err))
                  (begin
                    (set-box! edits (append (unbox edits) (lsp/format-edits res)))
                    (set-box! pending (- (unbox pending) 1))
                    (when (= (unbox pending) 0)
                      (lsp/format-apply! pane gen (unbox edits)))))))
          #:to server
          #:allow-stale #t))
      ranges)))

;; See docs/features.md, "Range requests".
(define (lsp/format-linewise! pane gen td ranges)
  (lsp/with-servers (lsp-servers pane #:method "textDocument/rangeFormatting") "range formatting"
    (lambda (servers)
      (let ((server (car servers))
            (n (length ranges))
            (cap (get-option "lsp.format-max-ranges")))
        (cond
          ((and (> n 1) (lsp/cap-flag? (lsp-capability server #:method "textDocument/rangeFormatting") "rangesSupport"))
           (lsp-request! pane "textDocument/rangesFormatting"
             (hash "textDocument" td "ranges" ranges "options" (lsp/format-options pane))
             (lsp/format-callback pane gen)
             #:to server
             #:allow-stale #t))
          ((> n cap)
           (log! 'info
                 (string-append (number->string n)
                                 " ranges exceeds lsp.format-max-ranges ("
                                 (number->string cap)
                                 ") — nothing formatted")))
          (else (lsp/format-fan-out! pane server gen td ranges)))))))

(define (lsp/format-source! pane)
  (let ((rp (lsp-linewise-ranges-params pane)))
    (if (not rp)
        (log! 'info "buffer has no path — nothing to format")
        (let* ((td (hash-ref rp "textDocument"))
               (ranges (hash-ref rp "ranges"))
               (gen (buffer-generation pane)))
          (cond
            ((selections-linewise? pane) (lsp/format-linewise! pane gen td ranges))
            ((selections-charwise? pane)
             (lsp-request! pane "textDocument/formatting"
               (hash "textDocument" td "options" (lsp/format-options pane))
               (lsp/format-callback pane gen)
               #:allow-stale #t))
            (else (log! 'info "mixed whole-line and partial selections — nothing formatted")))))))

(define-command! "lsp-fmt"
  "Format the buffer via LSP — bind this to a key, or call it from a hook (e.g. `on-buffer-save`)."
  (lambda (pane) (lsp/format-source! pane)))

(define-typed-command! "format-source"
  ":format-source — format the buffer via LSP from the command line."
  (lambda (pane) (lsp/format-source! pane)))
