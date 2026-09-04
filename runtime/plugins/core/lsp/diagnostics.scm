;;; core:lsp/diagnostics.scm — diagnostics navigation, EOL summary, gutter
;;; signs. No LSP request — reads the diagnostics store via
;;; diagnostics-for-buffer. See docs/decorations.md.

(require "lib.scm")

;; ── Helpers ──────────────────────────────────────────────────────────────────

(define (lsp/severity-glyph severity)
  (cond
    ((equal? severity "error") "✘")
    ((equal? severity "warning") "⚠")
    ((equal? severity "info") "ℹ")
    ((equal? severity "hint") "·")
    (else "?")))

(define (lsp/first-line text)
  (car (split-many text "\n")))

;; Wraps to the first/last entry overall when none qualifies — `diags` is
;; start-ascending.
(define (lsp/first-after diags head)
  (let ((after (filter (lambda (d) (> (hash-ref d "start") head)) diags)))
    (if (null? after) (car diags) (car after))))

(define (lsp/last-before diags head)
  (let ((before (filter (lambda (d) (< (hash-ref d "start") head)) diags)))
    (if (null? before) (car (reverse diags)) (car (reverse before)))))

(define (lsp/diag-jump-to! d)
  (goto-location! (list (current-buffer) (hash-ref d "line") (hash-ref d "char-col"))))

(define (lsp/diag-jump direction)
  (let ((diags (diagnostics-for-buffer (current-buffer))))
    (if (null? diags)
        (log! 'info "No diagnostics")
        (let* ((head (call! "stdlib/cursor-char-index" (current-selections)))
               (target (if (> direction 0)
                           (lsp/first-after diags head)
                           (lsp/last-before diags head))))
          (lsp/diag-jump-to! target)
          (show-popup! (hash-ref target "message") #:kind 'scrollable)))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "goto-next-diagnostic"
  "Jump to the next diagnostic after the cursor (wraps to the first)."
  (lambda () (lsp/diag-jump 1)))

(define-command! "goto-prev-diagnostic"
  "Jump to the previous diagnostic before the cursor (wraps to the last)."
  (lambda () (lsp/diag-jump -1)))

(define-typed-command! "diagnostics" ":diagnostics — list this buffer's diagnostics."
  (lambda ()
    (let ((diags (diagnostics-for-buffer (current-buffer))))
      (if (null? diags)
          (log! 'info "No diagnostics")
          (show-drawer-list!
            (map (lambda (d)
                   (string-append (lsp/severity-glyph (hash-ref d "severity")) " "
                                  (lsp/format-position (hash-ref d "line") (hash-ref d "grapheme-col")) " "
                                  (lsp/first-line (hash-ref d "message"))))
                 diags)
            (lambda (idx) (when idx (lsp/diag-jump-to! (list-ref diags idx)))))))))

;; ── Diagnostic decorations: EOL summary + gutter signs ──────────────────────
;; See docs/decorations.md.

;; Registration is per-buffer and idempotent — see docs/decorations.md.
(define lsp/*sign-priority* 10)

(define (lsp/severity-scope severity)
  (string-append "diagnostic." severity))

(define (lsp/most-severe line-diags)
  (foldl (lambda (d best)
           (if (< (hash-ref d "severity-rank") (hash-ref best "severity-rank")) d best))
         (car line-diags)
         (cdr line-diags)))

(define (lsp/group-by key-fn lst)
  (if (null? lst)
      '()
      (let loop ((rest (cdr lst))
                 (current-key (key-fn (car lst)))
                 (current-group (list (car lst)))
                 (groups '()))
        (cond
          ((null? rest)
           (reverse (cons (cons current-key (reverse current-group)) groups)))
          ((equal? (key-fn (car rest)) current-key)
           (loop (cdr rest) current-key (cons (car rest) current-group) groups))
          (else
           (loop (cdr rest) (key-fn (car rest)) (list (car rest))
                 (cons (cons current-key (reverse current-group)) groups)))))))

(define (lsp/group-by-line diags)
  (map cdr (lsp/group-by (lambda (d) (hash-ref d "line")) diags)))

(define (lsp/line-group->entry group)
  (let* ((leftmost (car group))
         (n (length group))
         (msg (lsp/first-line (hash-ref leftmost "message")))
         (body (if (> n 1) (string-append "[" (number->string n) "] " msg) msg))
         (text (string-append " " body))
         (scope (lsp/severity-scope (hash-ref (lsp/most-severe group) "severity"))))
    (list (hash-ref leftmost "line") text scope)))

(define (lsp/diag-line-pairs diag)
  (map (lambda (line) (cons line diag))
       (range (hash-ref diag "line") (+ (hash-ref diag "end-line") 1))))

(define (lsp/diagnostic-signs diags)
  (let* ((pairs (foldl (lambda (diag acc)
                          (foldl cons acc (lsp/diag-line-pairs diag)))
                        '()
                        diags))
         (sorted (sort pairs (lambda (a b) (< (car a) (car b)))))
         (groups (lsp/group-by car sorted)))
    (map (lambda (kv)
           (let ((line (car kv))
                 (line-diags (map cdr (cdr kv))))
             (list line "●" (hash-ref (lsp/most-severe line-diags) "severity"))))
         groups)))

(define (lsp/refresh-diagnostic-decorations bid)
  (register-sign-source! "lsp-diagnostics" bid lsp/*sign-priority*)
  (let ((diags (diagnostics-for-buffer bid)))
    (set-eol-text! "lsp-diagnostics" bid
      (map lsp/line-group->entry (lsp/group-by-line diags)))
    (set-signs! "lsp-diagnostics" bid (lsp/diagnostic-signs diags))))

(register-hook! 'on-diagnostics-changed
  (lambda (bid) (lsp/refresh-diagnostic-decorations bid)))

(register-hook! 'on-lsp-detach
  (lambda (bid server-name)
    (register-sign-source! "lsp-diagnostics" bid lsp/*sign-priority*)
    (set-eol-text! "lsp-diagnostics" bid '())
    (set-signs! "lsp-diagnostics" bid '())))

(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.diagnostics-severity-floor")
      (for-each lsp/refresh-diagnostic-decorations (buffers)))))
