;;; core:stdlib

;; ── Selection helpers (internal) ────────────────────────────────────────────
;;
;; Each selection is an opaque (anchor head primary?) triple — go through
;; these accessors, never car/cadr/caddr directly. Every helper passes #f
;; straight through, so callers check for #f once, at the call site.

;;; Anchor (0-indexed char offset) of a single selection triple, or #f.
(define (stdlib/selection-anchor sel)
  (and sel (car sel)))

;;; Head (0-indexed char offset) of a single selection triple, or #f.
(define (stdlib/selection-head sel)
  (and sel (cadr sel)))

;;; #t if a single selection triple is the primary selection, or #f.
(define (stdlib/selection-primary? sel)
  (and sel (caddr sel)))

;;; The selection triple flagged primary in `sels`, or #f.
(define (stdlib/primary-selection sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #f)
           ((stdlib/selection-primary? (car sels)) (car sels))
           (else (loop (cdr sels)))))))

;;; #t if `sels` holds exactly one selection (a single cursor), or #f.
(define (stdlib/single-selection? sels)
  (and sels (= (length sels) 1)))

;;; #t if every selection in `sels` spans a single grapheme (anchor == head),
;;; or #f.
(define (stdlib/all-single-char? sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #t)
           ((= (stdlib/selection-anchor (car sels)) (stdlib/selection-head (car sels)))
            (loop (cdr sels)))
           (else #f)))))

;;; The char index (0-indexed) of the primary cursor's head in `sels`, or #f.
(define (stdlib/cursor-char-index sels)
  (stdlib/selection-head (stdlib/primary-selection sels)))

;; ── call!-able commands (public API) ────────────────────────────────────────
;;
;; Command name and Steel binding of the same name live in separate
;; namespaces (command registry vs. module scope) — no collision between the
;; command "stdlib/all-single-char?" and the function it wraps.

(define-command! "stdlib/all-single-char?"
  "#t if every selection in the given list spans a single grapheme."
  (lambda (sels) (stdlib/all-single-char? sels)))

(define-command! "stdlib/single-selection?"
  "#t if the given selection list holds exactly one selection."
  (lambda (sels) (stdlib/single-selection? sels)))

(define-command! "stdlib/cursor-char-index"
  "0-indexed head char offset of the primary selection in the given list, or #f."
  (lambda (sels) (stdlib/cursor-char-index sels)))
