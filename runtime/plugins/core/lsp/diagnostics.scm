;;; core:lsp/diagnostics.scm — F4: diagnostics navigation. No LSP request —
;;; reads the C9 store via B5's diagnostics-for-buffer. Depends on
;;; core:stdlib for cursor-char-index (see plugin.scm).

(require "lib.scm")

;; ── Helpers ──────────────────────────────────────────────────────────────────

(define (lsp/severity-glyph severity)
  (cond
    ((equal? severity "error") "✗")
    ((equal? severity "warning") "⚠")
    ((equal? severity "info") "ℹ")
    ((equal? severity "hint") "·")
    (else "?")))

;;; First line of a (possibly multi-line) diagnostic message.
(define (lsp/first-line text)
  (car (split-many text "\n")))

;;; First entry whose "start" is strictly after `head` — "next" means the
;;; next diagnostic *starting* after the cursor, so a cursor sitting inside
;;; diagnostic A (whose start is at or before head) still advances to B, not
;;; back to A. Wraps to the first entry overall if none qualifies.
(define (lsp/first-after diags head)
  (let ((after (filter (lambda (d) (> (hash-ref d "start") head)) diags)))
    (if (null? after) (car diags) (car after))))

;;; Last entry whose "start" is strictly before `head`, wrapping to the last
;;; entry overall if none qualifies. `diags` is start-ascending (C9), so the
;;; last matching entry is the closest one before the cursor.
(define (lsp/last-before diags head)
  (let ((before (filter (lambda (d) (< (hash-ref d "start") head)) diags)))
    (if (null? before) (car (reverse diags)) (car (reverse before)))))

(define (lsp/diag-jump-to! d)
  (goto-location! (list (current-buffer) (hash-ref d "line") (hash-ref d "col"))))

(define (lsp/diag-jump direction)
  (let ((diags (diagnostics-for-buffer (current-buffer))))
    (if (null? diags)
        (log! 'info "No diagnostics")
        (let ((head (call! "stdlib/cursor-char-index" (current-selections))))
          (lsp/diag-jump-to!
            (if (> direction 0)
                (lsp/first-after diags head)
                (lsp/last-before diags head)))))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "goto-next-diagnostic"
  "Jump to the next diagnostic after the cursor (wraps to the first)."
  (lambda () (lsp/diag-jump 1)))

(define-command! "goto-prev-diagnostic"
  "Jump to the previous diagnostic before the cursor (wraps to the last)."
  (lambda () (lsp/diag-jump -1)))

(define-command! "diagnostics" ":diagnostics — list this buffer's diagnostics."
  (lambda ()
    (let ((diags (diagnostics-for-buffer (current-buffer))))
      (if (null? diags)
          (log! 'info "No diagnostics")
          (show-drawer-list!
            (map (lambda (d)
                   (string-append (lsp/severity-glyph (hash-ref d "severity")) " "
                                  (lsp/first-line (hash-ref d "message"))))
                 diags)
            (lambda (idx) (when idx (lsp/diag-jump-to! (list-ref diags idx)))))))))
