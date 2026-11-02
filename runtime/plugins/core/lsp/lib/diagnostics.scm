;;; core:lsp/diagnostics.scm — diagnostics navigation, drawer, EOL summary, gutter
;;; signs. See docs/decorations.md.

(require "helpers.scm")

;; ── Helpers ──────────────────────────────────────────────────────────────────

(define (lsp/severity-glyph severity)
  (cond
    ((equal? severity 'error) "✘")
    ((equal? severity 'warning) "⚠")
    ((equal? severity 'info) "ℹ")
    ((equal? severity 'hint) "·")
    (else "?")))

(define (lsp/first-line text)
  (let ((parts (split-once text "\n")))
    (if (pair? parts) (car parts) text)))

(define (lsp/first-after diags head)
  (or (call! "stdlib/find" (lambda (d) (> (hash-ref d 'start-char) head)) diags)
      (car diags)))

(define (lsp/last-before diags head)
  (or (foldl (lambda (d found) (if (< (hash-ref d 'start-char) head) d found)) #f diags)
      (last diags)))

(define (lsp/diag-jump-to! pane d)
  (lsp/jump-to! (hash 'target pane 'char-offset (hash-ref d 'start-char))))

;; `pick` is `lsp/first-after` or `lsp/last-before`.
(define (lsp/diag-jump! pane pick)
  (let ((diags (diagnostics-for-buffer pane)))
    (if (null? diags)
        (log! 'info "No diagnostics")
        (let ((target (pick diags (call! "stdlib/cursor-char-offset" (buffer-selections pane)))))
          (lsp/diag-jump-to! pane target)
          (show-popup! (focused-pane) (hash-ref target 'message) #:kind 'scrollable)))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "lsp/goto-next-diagnostic"
  "Jump to the next diagnostic after the cursor (wraps to the first)."
  (lambda (pane) (lsp/diag-jump! pane lsp/first-after)))

(define-command! "lsp/goto-prev-diagnostic"
  "Jump to the previous diagnostic before the cursor (wraps to the last)."
  (lambda (pane) (lsp/diag-jump! pane lsp/last-before)))

;; The tool the server names on the diagnostic, else the server's own name.
(define (lsp/diag-origin d)
  (let ((source (hash-ref d 'source)))
    (if (void? source) (lsp-server-name (hash-ref d 'server)) source)))

(define (lsp/diag-row d)
  (string-append (lsp/severity-glyph (hash-ref d 'severity)) " "
                 (lsp/format-position (hash-ref d 'start-line) (hash-ref d 'grapheme-col)) " "
                 (lsp/diag-origin d) ": "
                 (lsp/first-line (hash-ref d 'message))))

(define lsp/*diag-drawer* #f)

(define (lsp/diag-select-callback pane)
  (lambda (d drawer)
    (when d
      (lsp/diag-jump-to! pane d))))

(define-typed-command! "diagnostics" ":diagnostics — list this buffer's diagnostics."
  (lambda (pane)
    (let ((diags (diagnostics-for-buffer pane)))
      (if (null? diags)
          (log! 'info "No diagnostics")
          (let ((token (show-drawer-list! pane (lsp/payload-rows lsp/diag-row diags)
                                          (lsp/diag-select-callback pane))))
            (when token
              (set! lsp/*diag-drawer* (hash 'key (buffer-key pane) 'token token 'diags diags))))))))

(define (lsp/diag-best-match old new-diags)
  (let ((old-msg (hash-ref old 'message))
        (old-sev (hash-ref old 'severity))
        (old-line (hash-ref old 'start-line)))
    (let loop ((rest new-diags) (i 0) (best #f) (best-dist #f))
      (if (null? rest)
          best
          (let* ((d (car rest))
                 (dist (and (equal? (hash-ref d 'message) old-msg)
                            (equal? (hash-ref d 'severity) old-sev)
                            (abs (- (hash-ref d 'start-line) old-line)))))
            (if (and dist (or (not best-dist) (< dist best-dist)))
                (loop (cdr rest) (+ i 1) i dist)
                (loop (cdr rest) (+ i 1) best best-dist)))))))

;; The drawer's rows are `old-diags`, so `old-idx` names one of them.
(define (lsp/diag-refresh-index old-diags old-idx new-diags)
  (or (lsp/diag-best-match (list-ref old-diags old-idx) new-diags) old-idx))

(define (lsp/refresh-diagnostics-drawer! pane diags)
  (let ((drawer lsp/*diag-drawer*))
    (when (and drawer (equal? (buffer-key pane) (hash-ref drawer 'key)))
      (let ((token (hash-ref drawer 'token)))
        (cond
          ((null? diags) (close-drawer! token) (set! lsp/*diag-drawer* #f))
          ((lsp/refresh-drawer-list! token lsp/diag-row diags (lsp/diag-select-callback pane)
             (lambda (selected) (lsp/diag-refresh-index (hash-ref drawer 'diags) selected diags)))
           (set! lsp/*diag-drawer* (hash-insert drawer 'diags diags)))
          (else (set! lsp/*diag-drawer* #f)))))))

;; ── Diagnostic decorations: EOL summary + gutter signs ──────────────────────
;; See docs/decorations.md.

(define lsp/diagnostics-source "lsp-diagnostics")

(define lsp/*sign-priority* 10)

(define (lsp/severity-scope severity)
  (string-append (symbol->string severity) ".diagnostic.inline"))

(define (lsp/most-severe line-diags)
  (foldl (lambda (d best)
           (if (< (hash-ref d 'severity-rank) (hash-ref best 'severity-rank)) d best))
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
  (map cdr (lsp/group-by (lambda (d) (hash-ref d 'start-line)) diags)))

(define (lsp/line-group->entry group)
  (let* ((leftmost (car group))
         (n (length group))
         (msg (lsp/first-line (hash-ref leftmost 'message)))
         (body (if (> n 1) (string-append "[" (number->string n) "] " msg) msg))
         (text (string-append " " body))
         (scope (lsp/severity-scope (hash-ref (lsp/most-severe group) 'severity))))
    (hash 'line (hash-ref leftmost 'start-line) 'text text 'scope scope)))

(define (lsp/keep-more-severe worst line diag)
  (let ((held (hash-try-get worst line)))
    (if (and held (<= (hash-ref held 'severity-rank) (hash-ref diag 'severity-rank)))
        worst
        (hash-insert worst line diag))))

(define (lsp/diagnostic-signs diags)
  (let ((worst (foldl (lambda (diag worst)
                        (foldl (lambda (line worst) (lsp/keep-more-severe worst line diag))
                               worst
                               (range (hash-ref diag 'start-line) (hash-ref diag 'end-line))))
                      (hash)
                      diags)))
    (map (lambda (kv)
           (hash 'line (car kv) 'text "●" 'scope (symbol->string (hash-ref (cdr kv) 'severity))))
         (hash->list worst))))

(define (lsp/refresh-diagnostic-decorations! pane diags)
  (register-sign-source! pane lsp/diagnostics-source lsp/*sign-priority*)
  (set-eol-text! pane lsp/diagnostics-source
    (map lsp/line-group->entry (lsp/group-by-line diags))
    #:hide-on-insert-line (not (get-option "lsp-diagnostics-on-insert-line")))
  (set-signs! pane lsp/diagnostics-source (lsp/diagnostic-signs diags)))

(define (lsp/refresh-diagnostics! pane)
  (let ((diags (diagnostics-for-buffer pane)))
    (lsp/refresh-diagnostic-decorations! pane diags)
    (lsp/refresh-diagnostics-drawer! pane diags)))

(register-hook! 'on-diagnostics-changed lsp/refresh-diagnostics!)

(register-hook! 'on-option-change
  (lambda (key value)
    (when (member key '("lsp-diagnostics-min-severity" "lsp-diagnostics-on-insert-line"))
      (for-each lsp/refresh-diagnostics! (buffers)))))
