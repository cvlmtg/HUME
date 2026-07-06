;;; core:stdlib — library of helper functions for plugin authors.
;;;
;;; Scripting layers:
;;;   - BOOTSTRAP (hume-scripting/src/builtins/mod.rs) — core dispatch
;;;     primitives (call!, load-plugin, define-command!); always available.
;;;   - prelude (runtime/scheme/prelude.scm) — convenience macros for
;;;     init.scm; loaded at startup when the runtime dir exists.
;;;   - core:stdlib (this plugin) — functions useful to plugin authors;
;;;     loaded explicitly via (load-plugin "core:stdlib") in init.scm,
;;;     before any plugin that depends on it.

(provide stdlib/selection-anchor stdlib/selection-head stdlib/selection-primary?
         stdlib/primary-selection stdlib/single-selection? stdlib/all-single-char?
         stdlib/cursor-char-index)

;; ── Selection helpers ─────────────────────────────────────────────────────────
;;
;; Operate on the list returned by `(current-selections)`: each selection is an
;; opaque `(anchor head primary?)` triple — always go through these accessors,
;; never `car`/`cadr`/`caddr` directly on a selection in plugin code. Every
;; helper here passes `#f` straight through, so callers only need to check for
;; `#f` once, at the `(current-selections)` call site.

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
