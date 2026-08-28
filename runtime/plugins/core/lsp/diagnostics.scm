;;; core:lsp/diagnostics.scm — diagnostics navigation. No LSP request —
;;; reads the diagnostics store via diagnostics-for-buffer. See README.md
;;; "How it works" → "Diagnostics".

(require "lib.scm")

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

;;; First entry whose "start" is strictly after `head` — a cursor sitting
;;; inside diagnostic A still advances to B, not back to A. Wraps to the
;;; first entry overall if none qualifies.
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
  (goto-location! (list (current-buffer) (hash-ref d "line") (hash-ref d "char-col"))))

;;; gn/gp only — jumps like `lsp/diag-jump-to!`, then pops the target's full
;;; message in a dismiss-on-any-key overlay (Ctrl+u/d scroll it instead).
;;; `:diagnostics`' drawer-select calls `lsp/diag-jump-to!` directly, no popup.
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

(define-command! "diagnostics" ":diagnostics — list this buffer's diagnostics."
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
;; One "[n] <message>" appended after each offending line: text from the
;; leftmost diagnostic, color from the most severe one on that line. One
;; gutter sign per line any diagnostic touches, most severe wins.

;;; Fixed priority every diagnostic sign carries, regardless of severity —
;;; matches the Rust sign pass this replaced (`DIAGNOSTIC_SIGN_PRIORITY`).
(define lsp/*diagnostic-sign-priority* 10)

(define (lsp/severity-scope severity)
  (string-append "diagnostic." severity))

;;; The most severe entry in `line-diags`, by `"severity-rank"` — the
;;; discriminant `DiagSeverity`'s `Ord` assigns in Rust (0 = error … 3 =
;;; hint, lower is more severe), carried on every `diagnostics-for-buffer`
;;; entry so this is the only place severity order is compared, not
;;; re-encoded here.
(define (lsp/most-severe line-diags)
  (car (sort line-diags
             (lambda (a b) (< (hash-ref a "severity-rank") (hash-ref b "severity-rank"))))))

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

;;; One group -> a `(line text scope)` entry for `set-eol-text!`.
(define (lsp/line-group->entry group)
  (let* ((leftmost (car group))
         (n (length group))
         (msg (lsp/first-line (hash-ref leftmost "message")))
         (body (if (> n 1) (string-append "[" (number->string n) "] " msg) msg))
         (text (string-append " " body))
         (scope (lsp/severity-scope (hash-ref (lsp/most-severe group) "severity"))))
    (list (hash-ref leftmost "line") text scope)))

;;; `(line . diag)` pairs, one per line `diag`'s range touches (`"line"`
;;; through `"end-line"`, inclusive) — a diagnostic crossing several lines
;;; contributes a sign candidate to each one, the same span the Rust sign
;;; pass this replaced expanded before every render.
(define (lsp/diag-line-pairs diag)
  (map (lambda (line) (cons line diag))
       (range (hash-ref diag "line") (+ (hash-ref diag "end-line") 1))))

;;; `pairs` grouped by ascending `car` (line) into `(line . diags)` groups —
;;; the sign-side counterpart to `lsp/group-by-line`, needed separately
;;; because one diagnostic can land in more than one line's group here.
(define (lsp/pairs->line-groups pairs)
  (let ((sorted (sort pairs (lambda (a b) (< (car a) (car b))))))
    (if (null? sorted)
        '()
        (let loop ((rest (cdr sorted))
                   (current-line (car (car sorted)))
                   (current-group (list (cdr (car sorted))))
                   (groups '()))
          (cond
            ((null? rest)
             (reverse (cons (cons current-line (reverse current-group)) groups)))
            ((equal? (car (car rest)) current-line)
             (loop (cdr rest) current-line (cons (cdr (car rest)) current-group) groups))
            (else
             (loop (cdr rest) (car (car rest)) (list (cdr (car rest)))
                   (cons (cons current-line (reverse current-group)) groups))))))))

;;; `diags` -> `(line "●" severity priority)` sign entries, one per line any
;;; diagnostic touches — the most severe diagnostic on a line wins, same
;;; reduction `lsp/most-severe` already does for the EOL summary. `severity`
;;; is passed straight through as the sign's scope — the bare
;;; `error`/`warning`/`info`/`hint` name, Helix's own gutter-scope
;;; convention (distinct from `lsp/severity-scope`'s `diagnostic.*` prefix,
;;; which is for the editing-area text span, not the gutter).
(define (lsp/diagnostic-signs diags)
  (map (lambda (group)
         (list (car group) "●" (hash-ref (lsp/most-severe (cdr group)) "severity")
               lsp/*diagnostic-sign-priority*))
       (lsp/pairs->line-groups (apply append (map lsp/diag-line-pairs diags)))))

(define (lsp/refresh-diagnostic-decorations bid)
  (let ((diags (diagnostics-for-buffer bid)))
    (set-eol-text! "lsp-diagnostics" bid
      (map lsp/line-group->entry (lsp/group-by-line diags)))
    (set-signs! "lsp-diagnostics" bid (lsp/diagnostic-signs diags))))

(register-hook! 'on-diagnostics-changed
  (lambda (bid) (lsp/refresh-diagnostic-decorations bid)))

;;; A detached server means nothing new from diagnostics-for-buffer — clear
;;; both decorations explicitly rather than let the last summary/signs sit
;;; rendered forever.
(register-hook! 'on-lsp-detach
  (lambda (bid server-name)
    (set-eol-text! "lsp-diagnostics" bid '())
    (set-signs! "lsp-diagnostics" bid '())))

;;; A severity-floor change needs an explicit refresh — see README.
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.diagnostics-severity-floor")
      (for-each lsp/refresh-diagnostic-decorations (buffers)))))
