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

;; `split-once`, not `split-many`: a multi-line rustc message is routinely
;; 5-20 lines, and every row is rebuilt on every publish — splitting the
;; whole message just to keep the first line allocates and discards the
;; rest for nothing. `split-once` answers `#t` (not `#f`) when the pattern
;; isn't found, so `pair?` — not truthiness — is what tells "found a split"
;; from "no newline in this message" apart; a bare `(if parts (car parts)
;; text)` would call `(car #t)` on every single-line message, which is most
;; of them.
(define (lsp/first-line text)
  (let ((parts (split-once text "\n")))
    (if (pair? parts) (car parts) text)))

;; Wraps to the first/last entry overall when none qualifies — `diags` is
;; start-ascending.
(define (lsp/first-after diags head)
  (let ((after (filter (lambda (d) (> (hash-ref d "start") head)) diags)))
    (if (null? after) (car diags) (car after))))

(define (lsp/last-before diags head)
  (let ((before (filter (lambda (d) (< (hash-ref d "start") head)) diags)))
    (if (null? before) (car (reverse diags)) (car (reverse before)))))

;; `bid` is explicit rather than `(current-buffer)` — the diagnostics drawer
;; stays open across a buffer switch by design (browse-while-editing), so a
;; row selected there must jump into the buffer it was listed for, not
;; whichever buffer happens to be focused when Enter is pressed.
(define (lsp/diag-jump-to! bid d)
  (goto-location! (list bid (hash-ref d "line") (hash-ref d "char-col"))))

(define (lsp/diag-jump direction)
  (let ((diags (diagnostics-for-buffer (current-buffer))))
    (if (null? diags)
        (log! 'info "No diagnostics")
        (let* ((head (call! "stdlib/cursor-char-index" (current-selections)))
               (target (if (> direction 0)
                           (lsp/first-after diags head)
                           (lsp/last-before diags head))))
          (lsp/diag-jump-to! (current-buffer) target)
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
;; its own drawer: it tracks the open drawer itself (buffer + token + last
;; snapshot) and rebuilds rows on every `on-diagnostics-changed` for its
;; buffer. `#f` when closed, `(bid tok diags)` when open — one value, not
;; three hand-synced globals, so "closed" is structural rather than an
;; invariant across three separately-cleared fields. Tracking clears through
;; `update-drawer-list!`/`close-drawer!` no-oping in
;; `lsp/refresh-diagnostics-drawer` below (drawer closed, replaced, or
;; belonging to someone else — Rust checks the token before touching
;; anything, so this plugin can never reach a foreign drawer even by
;; mistake); an `Esc` or a foreign replace is caught the same way, lazily, on
;; the next publish rather than eagerly through the callback's own `#f` —
;; every path that could fire `#f` already routes through here on its next
;; visit, and every mutator below is already token-guarded, so a `#f` this
;; plugin never sees can't do anything wrong in the meantime.
(define lsp/*diag-drawer* #f)

(define (lsp/diag-select-callback bid diags)
  (lambda (idx)
    (when idx
      (lsp/diag-jump-to! bid (list-ref diags idx)))))

(define-typed-command! "diagnostics" ":diagnostics — list this buffer's diagnostics."
  (lambda ()
    (let ((diags (diagnostics-for-buffer (current-buffer))))
      (if (null? diags)
          (log! 'info "No diagnostics")
          (let* ((bid (current-buffer))
                 (tok (show-drawer-list! (map lsp/diag-row diags)
                                         (lsp/diag-select-callback bid diags))))
            ;; `#f` (a stale async open — see `show-drawer-list!`'s own doc)
            ;; leaves tracking untouched: there is no drawer to track.
            (when tok
              (set! lsp/*diag-drawer* (list bid tok diags))))))))

;; The nearest surviving match for OLD in NEW-DIAGS by message + severity —
;; position stays out of the key on purpose, since the fix's own edit can
;; shift other diagnostics' lines. Ties (the same message twice) break
;; toward the nearest line. `#f` when nothing matches.
(define (lsp/diag-best-match old new-diags)
  (let ((old-msg (hash-ref old "message"))
        (old-sev (hash-ref old "severity"))
        (old-line (hash-ref old "line")))
    (let loop ((rest new-diags) (i 0) (best #f) (best-dist #f))
      (if (null? rest)
          best
          (let ((d (car rest)))
            (if (and (equal? (hash-ref d "message") old-msg)
                     (equal? (hash-ref d "severity") old-sev))
                (let ((dist (abs (- (hash-ref d "line") old-line))))
                  (if (or (not best-dist) (< dist best-dist))
                      (loop (cdr rest) (+ i 1) i dist)
                      (loop (cdr rest) (+ i 1) best best-dist)))
                (loop (cdr rest) (+ i 1) best best-dist)))))))

;; Returns the index into NEW-DIAGS (guaranteed non-empty — the sole caller
;; guards on `(null? diags)` first) to select on a refresh: the surviving
;; diagnostic ([`lsp/diag-best-match`]), or the old position clamped — the
;; item now at that position, i.e. the next one when the selected diagnostic
;; itself was fixed.
(define (lsp/diag-refresh-index old-diags old-idx new-diags)
  (let ((fallback (min old-idx (- (length new-diags) 1)))
        (old (and (< old-idx (length old-diags)) (list-ref old-diags old-idx))))
    (if old (or (lsp/diag-best-match old new-diags) fallback) fallback)))

;; `diags` is fetched once by the caller (the hooks below, each of which also
;; needs it for decorations) rather than re-fetched here — `diagnostics-for-buffer`
;; sorts and deep-clones up to 1000 diagnostics' whole raw LSP JSON, so
;; fetching it twice per publish would double that cost for nothing.
(define (lsp/refresh-diagnostics-drawer bid diags)
  ;; `lsp/diag-refresh-index` and `update-drawer-list!` both clamp into the
  ;; new list, so `sel` passes through raw — no Scheme-side clamp. `tok`
  ;; passes to every call below — Rust ignores any of them the moment `tok`
  ;; no longer names the open drawer (closed, replaced, or never ours), so
  ;; `close-drawer!` here can never reach a foreign drawer the way it could
  ;; before the token existed.
  (when (and lsp/*diag-drawer* (equal? bid (car lsp/*diag-drawer*)))
    (let ((tok (cadr lsp/*diag-drawer*)))
      (if (null? diags)
          (begin (close-drawer! tok) (set! lsp/*diag-drawer* #f))
          (let ((sel (drawer-selected-index tok)))
            (if (not sel)
                (set! lsp/*diag-drawer* #f)
                (let ((idx (lsp/diag-refresh-index (caddr lsp/*diag-drawer*) sel diags)))
                  ;; An update replaces the callback silently (no `#f`), and
                  ;; keeps the drawer's existing token.
                  (if (update-drawer-list! tok (map lsp/diag-row diags)
                                           (lsp/diag-select-callback bid diags)
                                           idx)
                      (set! lsp/*diag-drawer* (list bid tok diags))
                      (set! lsp/*diag-drawer* #f)))))))))

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

;; `diags` comes from the caller — see `lsp/refresh-diagnostics-drawer`'s own
;; doc for why: every caller here already needs its own copy for decorations
;; or the drawer refresh, so fetching it a second time inside this function
;; would double the cost of `diagnostics-for-buffer`'s sort-and-deep-clone
;; for nothing.
(define (lsp/refresh-diagnostic-decorations bid diags)
  (register-sign-source! "lsp-diagnostics" bid lsp/*sign-priority*)
  (set-eol-text! "lsp-diagnostics" bid
    (map lsp/line-group->entry (lsp/group-by-line diags)))
  (set-signs! "lsp-diagnostics" bid (lsp/diagnostic-signs diags)))

(register-hook! 'on-diagnostics-changed
  (lambda (bid)
    (let ((diags (diagnostics-for-buffer bid)))
      (lsp/refresh-diagnostic-decorations bid diags)
      (lsp/refresh-diagnostics-drawer bid diags))))

(register-hook! 'on-lsp-detach
  (lambda (bid server-name)
    ;; Detach always clears to empty — no fetch needed, unlike the other two
    ;; hooks, whose whole point is a diagnostic set that just changed.
    (register-sign-source! "lsp-diagnostics" bid lsp/*sign-priority*)
    (set-eol-text! "lsp-diagnostics" bid '())
    (set-signs! "lsp-diagnostics" bid '())
    (lsp/refresh-diagnostics-drawer bid '())))

(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.diagnostics-severity-floor")
      (for-each (lambda (bid)
                  (let ((diags (diagnostics-for-buffer bid)))
                    (lsp/refresh-diagnostic-decorations bid diags)
                    (lsp/refresh-diagnostics-drawer bid diags)))
                (buffers)))))
