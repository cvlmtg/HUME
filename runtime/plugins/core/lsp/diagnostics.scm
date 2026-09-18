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

(define (lsp/diag-row d)
  (string-append (lsp/severity-glyph (hash-ref d "severity")) " "
                 (lsp/format-position (hash-ref d "line") (hash-ref d "grapheme-col")) " "
                 (lsp/first-line (hash-ref d "message"))))

;; The drawer freezes its rows at open time, so this plugin owns refreshing
;; its own drawer: it tracks the open drawer itself (buffer + generation +
;; last snapshot) and rebuilds rows on every `on-diagnostics-changed` for
;; its buffer. Openness is the buffer alone (`#f` = closed) — no separate
;; flag to keep in sync with it. Tracking clears through the callback's
;; `#f` — `Esc`, or a replace by another drawer's `show-drawer-list!`,
;; which fires `#f` to the outgoing callback — and through
;; `update-drawer-list!` reporting `#f` (drawer closed or replaced before
;; its `#f` drained: an expected-normal race, never an error). The
;; generation keeps a replace's own stale `#f` from killing the fresh
;; drawer: only the current generation may clear. A pure liveness check
;; (`drawer-selected-index` answering non-`#f`) can't replace the
;; generation: a *foreign* replace's `#f` drains while that foreign drawer
;; is open, so liveness would keep tracking alive and the next publish
;; would refresh someone else's rows.
(define lsp/*diagnostics-drawer-bid* #f)
(define lsp/*diagnostics-drawer-gen* 0)
(define lsp/*diagnostics-drawer-diags* '())

(define (lsp/diag-select-callback gen diags)
  (lambda (idx)
    (if idx
        (lsp/diag-jump-to! (list-ref diags idx))
        (when (= gen lsp/*diagnostics-drawer-gen*)
          (set! lsp/*diagnostics-drawer-bid* #f)))))

(define-typed-command! "diagnostics" ":diagnostics — list this buffer's diagnostics."
  (lambda ()
    (let ((diags (diagnostics-for-buffer (current-buffer))))
      (if (null? diags)
          (log! 'info "No diagnostics")
          (begin
            (set! lsp/*diagnostics-drawer-gen* (+ lsp/*diagnostics-drawer-gen* 1))
            (let ((gen lsp/*diagnostics-drawer-gen*)
                  (bid (current-buffer)))
              (show-drawer-list! (map lsp/diag-row diags)
                                 (lsp/diag-select-callback gen diags))
              (set! lsp/*diagnostics-drawer-bid* bid)
              (set! lsp/*diagnostics-drawer-diags* diags)))))))

;; Same diagnostic across a refresh, by message + severity — position stays
;; out of the key on purpose, since the fix's own edit can shift other
;; diagnostics' lines. Ties (the same message twice) break toward the
;; nearest line. Returns the index into NEW-DIAGS to select: the surviving
;; diagnostic, or the old position clamped — the item now at that position,
;; i.e. the next one when the selected diagnostic itself was fixed.
(define (lsp/diag-refresh-index old-diags old-idx new-diags)
  (let ((n (length new-diags)))
    (if (= n 0)
        0
        (let ((old (if (< old-idx (length old-diags)) (list-ref old-diags old-idx) #f)))
          (if (not old)
              (min old-idx (- n 1))
              (let loop ((rest new-diags) (i 0) (best #f) (best-dist #f))
                (if (null? rest)
                    (if best best (min old-idx (- n 1)))
                    (let ((d (car rest)))
                      (if (and (equal? (hash-ref d "message") (hash-ref old "message"))
                               (equal? (hash-ref d "severity") (hash-ref old "severity")))
                          (let ((dist (abs (- (hash-ref d "line") (hash-ref old "line")))))
                            (if (or (not best-dist) (< dist best-dist))
                                (loop (cdr rest) (+ i 1) i dist)
                                (loop (cdr rest) (+ i 1) best best-dist)))
                          (loop (cdr rest) (+ i 1) best best-dist))))))))))

(define (lsp/refresh-diagnostics-drawer bid)
  ;; `lsp/diag-refresh-index` and `update-drawer-list!` both clamp into the
  ;; new list, so `sel` passes through raw — no Scheme-side clamp.
  (when (and lsp/*diagnostics-drawer-bid*
             (equal? bid lsp/*diagnostics-drawer-bid*))
    (let ((diags (diagnostics-for-buffer bid)))
      (if (null? diags)
          (begin (close-drawer!) (set! lsp/*diagnostics-drawer-bid* #f))
          (let ((sel (drawer-selected-index)))
            (if (not sel)
                (set! lsp/*diagnostics-drawer-bid* #f)
                (let ((idx (lsp/diag-refresh-index lsp/*diagnostics-drawer-diags* sel diags)))
                  ;; An update replaces the callback silently (no `#f`), so
                  ;; no generation bump: the new closure is current by
                  ;; construction.
                  (if (update-drawer-list! (map lsp/diag-row diags)
                                           (lsp/diag-select-callback lsp/*diagnostics-drawer-gen* diags)
                                           idx)
                      (set! lsp/*diagnostics-drawer-diags* diags)
                      (set! lsp/*diagnostics-drawer-bid* #f)))))))))

;; ── Diagnostic decorations: EOL summary + gutter signs ──────────────────────
;; See docs/decorations.md.

;; Registration is per-buffer and idempotent — see docs/decorations.md.
(define lsp/*sign-priority* 10)

(define (lsp/severity-scope severity)
  (string-append severity ".diagnostic.inline"))

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
  (lambda (bid)
    (lsp/refresh-diagnostic-decorations bid)
    (lsp/refresh-diagnostics-drawer bid)))

(register-hook! 'on-lsp-detach
  (lambda (bid server-name)
    (register-sign-source! "lsp-diagnostics" bid lsp/*sign-priority*)
    (set-eol-text! "lsp-diagnostics" bid '())
    (set-signs! "lsp-diagnostics" bid '())
    (lsp/refresh-diagnostics-drawer bid)))

(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.diagnostics-severity-floor")
      (for-each (lambda (bid)
                  (lsp/refresh-diagnostic-decorations bid)
                  (lsp/refresh-diagnostics-drawer bid))
                (buffers)))))
