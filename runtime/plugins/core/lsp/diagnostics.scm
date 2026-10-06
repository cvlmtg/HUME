;;; core:lsp/diagnostics.scm — diagnostics navigation, drawer, EOL summary, gutter
;;; signs. See docs/decorations.md.

(require "lib.scm")

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
  (let ((after (filter (lambda (d) (> (hash-ref d 'start) head)) diags)))
    (if (null? after) (car diags) (car after))))

(define (lsp/last-before diags head)
  (let ((before (filter (lambda (d) (< (hash-ref d 'start) head)) diags)))
    (if (null? before) (car (reverse diags)) (car (reverse before)))))

(define (lsp/diag-jump-to! pane d)
  (goto-location! (focused-pane)
                  (hash 'target pane 'line (hash-ref d 'line) 'char-col (hash-ref d 'char-col))))

(define (lsp/diag-jump! pane direction)
  (let ((diags (diagnostics-for-buffer pane)))
    (if (null? diags)
        (log! 'info "No diagnostics")
        (let* ((head (call! "stdlib/cursor-char-index" (buffer-selections pane)))
               (target (if (> direction 0)
                           (lsp/first-after diags head)
                           (lsp/last-before diags head))))
          (lsp/diag-jump-to! pane target)
          (show-popup! (focused-pane) (hash-ref target 'message) #:kind 'scrollable)))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "goto-next-diagnostic"
  "Jump to the next diagnostic after the cursor (wraps to the first)."
  (lambda (pane) (lsp/diag-jump! pane 1)))

(define-command! "goto-prev-diagnostic"
  "Jump to the previous diagnostic before the cursor (wraps to the last)."
  (lambda (pane) (lsp/diag-jump! pane -1)))

(define (lsp/diag-row d)
  (string-append (lsp/severity-glyph (hash-ref d 'severity)) " "
                 (lsp/format-position (hash-ref d 'line) (hash-ref d 'grapheme-col)) " "
                 (lsp-server-name (hash-ref d 'server)) ": "
                 (lsp/first-line (hash-ref d 'message))))

(define lsp/*diag-drawer* #f)

(define (lsp/diag-select-callback pane diags)
  (lambda (idx drawer)
    (when idx
      (lsp/diag-jump-to! pane (list-ref diags idx)))))

(define-typed-command! "diagnostics" ":diagnostics — list this buffer's diagnostics."
  (lambda (pane)
    (let ((diags (diagnostics-for-buffer pane)))
      (if (null? diags)
          (log! 'info "No diagnostics")
          (let* ((tok (show-drawer-list! pane (map lsp/diag-row diags)
                                         (lsp/diag-select-callback pane diags))))
            (when tok
              (set! lsp/*diag-drawer* (list (buffer-key pane) tok diags))))))))

(define (lsp/diag-best-match old new-diags)
  (let ((old-msg (hash-ref old 'message))
        (old-sev (hash-ref old 'severity))
        (old-line (hash-ref old 'line)))
    (let loop ((rest new-diags) (i 0) (best #f) (best-dist #f))
      (if (null? rest)
          best
          (let ((d (car rest)))
            (if (and (equal? (hash-ref d 'message) old-msg)
                     (equal? (hash-ref d 'severity) old-sev))
                (let ((dist (abs (- (hash-ref d 'line) old-line))))
                  (if (or (not best-dist) (< dist best-dist))
                      (loop (cdr rest) (+ i 1) i dist)
                      (loop (cdr rest) (+ i 1) best best-dist)))
                (loop (cdr rest) (+ i 1) best best-dist)))))))

(define (lsp/diag-refresh-index old-diags old-idx new-diags)
  (let ((fallback (min old-idx (- (length new-diags) 1)))
        (old (and (< old-idx (length old-diags)) (list-ref old-diags old-idx))))
    (if old (or (lsp/diag-best-match old new-diags) fallback) fallback)))

(define (lsp/refresh-diagnostics-drawer! pane diags)
  (when (and lsp/*diag-drawer* (equal? (buffer-key pane) (car lsp/*diag-drawer*)))
    (let ((tok (cadr lsp/*diag-drawer*)))
      (if (null? diags)
          (begin (close-drawer! tok) (set! lsp/*diag-drawer* #f))
          (let ((sel (drawer-selected-index tok)))
            (if (not sel)
                (set! lsp/*diag-drawer* #f)
                (let ((idx (lsp/diag-refresh-index (caddr lsp/*diag-drawer*) sel diags)))
                  (if (update-drawer-list! tok (map lsp/diag-row diags)
                                           (lsp/diag-select-callback pane diags)
                                           idx)
                      (set! lsp/*diag-drawer* (list (buffer-key pane) tok diags))
                      (set! lsp/*diag-drawer* #f)))))))))

;; ── Diagnostic decorations: EOL summary + gutter signs ──────────────────────
;; See docs/decorations.md.

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
  (map cdr (lsp/group-by (lambda (d) (hash-ref d 'line)) diags)))

(define (lsp/line-group->entry group)
  (let* ((leftmost (car group))
         (n (length group))
         (msg (lsp/first-line (hash-ref leftmost 'message)))
         (body (if (> n 1) (string-append "[" (number->string n) "] " msg) msg))
         (text (string-append " " body))
         (scope (lsp/severity-scope (hash-ref (lsp/most-severe group) 'severity))))
    (hash 'line (hash-ref leftmost 'line) 'text text 'scope scope)))

(define (lsp/diag-line-pairs diag)
  (map (lambda (line) (cons line diag))
       (range (hash-ref diag 'line) (+ (hash-ref diag 'end-line) 1))))

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
             (hash 'line line 'text "●" 'scope (symbol->string (hash-ref (lsp/most-severe line-diags) 'severity)))))
         groups)))

(define (lsp/refresh-diagnostic-decorations! pane diags)
  (register-sign-source! "lsp-diagnostics" pane lsp/*sign-priority*)
  (set-eol-text! "lsp-diagnostics" pane
    (map lsp/line-group->entry (lsp/group-by-line diags))
    #:hide-on-insert-line (not (get-option "lsp.diagnostics-on-insert-line")))
  (set-signs! "lsp-diagnostics" pane (lsp/diagnostic-signs diags)))

(define (lsp/refresh-diagnostics! pane)
  (let ((diags (diagnostics-for-buffer pane)))
    (lsp/refresh-diagnostic-decorations! pane diags)
    (lsp/refresh-diagnostics-drawer! pane diags)))

(register-hook! 'on-diagnostics-changed lsp/refresh-diagnostics!)

;; A detached server's diagnostics are already gone from the store; what the
;; buffer's other servers published stays.
(register-hook! 'on-lsp-detach
  (lambda (pane server) (lsp/refresh-diagnostics! pane)))

(register-hook! 'on-option-change
  (lambda (key value)
    (when (member key '("lsp.diagnostics-severity-floor" "lsp.diagnostics-on-insert-line"))
      (for-each lsp/refresh-diagnostics! (buffers)))))
