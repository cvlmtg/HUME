;;; core:lsp/diagnostics.scm — diagnostics navigation. No LSP request —
;;; reads the diagnostics store via diagnostics-for-buffer. Depends on
;;; core:stdlib for cursor-char-index (see plugin.scm).

;; ── Helpers ──────────────────────────────────────────────────────────────────

(define (lsp/severity-glyph severity)
  (cond
    ((equal? severity "error") "✘")
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
;;; entry overall if none qualifies. `diags` is start-ascending, so the
;;; last matching entry is the closest one before the cursor.
(define (lsp/last-before diags head)
  (let ((before (filter (lambda (d) (< (hash-ref d "start") head)) diags)))
    (if (null? before) (car (reverse diags)) (car (reverse before)))))

(define (lsp/diag-jump-to! d)
  (goto-location! (list (current-buffer) (hash-ref d "line") (hash-ref d "col"))))

;;; gn/gp only — jumps like `lsp/diag-jump-to!`, then pops the target's full
;;; message in a dismiss-on-next-key overlay. `:diagnostics`' drawer-select
;;; callback calls `lsp/diag-jump-to!` directly and stays overlay-free.
(define (lsp/diag-jump direction)
  (let ((diags (diagnostics-for-buffer (current-buffer))))
    (if (null? diags)
        (log! 'info "No diagnostics")
        (let* ((head (call! "stdlib/cursor-char-index" (current-selections)))
               (target (if (> direction 0)
                           (lsp/first-after diags head)
                           (lsp/last-before diags head))))
          (lsp/diag-jump-to! target)
          (show-popup! (hash-ref target "message") #:dismiss-on-key #t)))))

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

;; ── End-of-line inline summary ──────────────────────────────────────────────
;;
;; One "[n] <message>" (or bare message when n=1) appended after each
;; offending line's code, re-rendered on every on-diagnostics-changed batch.
;; Message text comes from the line's leftmost (smallest "col") diagnostic;
;; color comes from the line's most severe one — a line with any error reads
;; red even when the leftmost message is a warning.

;;; Lower rank = more severe. Unranked/unknown severities sort last.
(define (lsp/severity-rank severity)
  (cond
    ((equal? severity "error") 0)
    ((equal? severity "warning") 1)
    ((equal? severity "info") 2)
    ((equal? severity "hint") 3)
    (else 4)))

(define (lsp/severity-scope severity)
  (string-append "diagnostic." severity))

;;; The most severe entry in `line-diags` — only its severity is read by the
;;; caller, so ties (two entries at the same severity) are inconsequential.
(define (lsp/most-severe line-diags)
  (car (sort line-diags
             (lambda (a b) (< (lsp/severity-rank (hash-ref a "severity"))
                               (lsp/severity-rank (hash-ref b "severity")))))))

;;; `diags` (start-ascending, so same-line entries are already contiguous) ->
;;; a list of same-"line" groups, each group itself start-ascending.
(define (lsp/group-by-line diags)
  (if (null? diags)
      '()
      (let loop ((rest (cdr diags))
                 (current-line (hash-ref (car diags) "line"))
                 (current-group (list (car diags)))
                 (groups '()))
        (cond
          ((null? rest)
           (reverse (cons (reverse current-group) groups)))
          ((equal? (hash-ref (car rest) "line") current-line)
           (loop (cdr rest) current-line (cons (car rest) current-group) groups))
          (else
           (loop (cdr rest) (hash-ref (car rest) "line") (list (car rest))
                 (cons (reverse current-group) groups)))))))

;;; One group -> a `(line text scope)` entry for `set-inline-diagnostics!`.
(define (lsp/line-group->entry group)
  (let* ((leftmost (car group))
         (n (length group))
         (msg (lsp/first-line (hash-ref leftmost "message")))
         (body (if (> n 1) (string-append "[" (number->string n) "] " msg) msg))
         (text (string-append " " body))
         (scope (lsp/severity-scope (hash-ref (lsp/most-severe group) "severity"))))
    (list (hash-ref leftmost "line") text scope)))

(define (lsp/refresh-inline-diagnostics bid)
  (let ((diags (diagnostics-for-buffer bid)))
    (set-inline-diagnostics! bid
      (map lsp/line-group->entry (lsp/group-by-line diags)))))

(register-hook! 'on-diagnostics-changed
  (lambda (bid) (lsp/refresh-inline-diagnostics bid)))

;;; Once `bid` has no attached server, diagnostics-for-buffer returns nothing
;;; new — clear explicitly rather than let a detached server's last inline
;;; summary sit rendered forever (same reasoning as inlay hints' on-lsp-detach).
(register-hook! 'on-lsp-detach
  (lambda (bid server-name) (set-inline-diagnostics! bid '())))

;; ── gn/gp full-message overlay ───────────────────────────────────────────────
;;
;; Jumping via goto-next-diagnostic/goto-prev-diagnostic additionally pops
;; the jumped-to diagnostic's full (possibly multi-line) message in a
;; transient popup, dismissed by Editor::handle_key on the next key press —
;; unlike hover/signature-help, which dismiss only on-mode-change.
