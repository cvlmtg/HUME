;;; runtime/scheme/prelude.scm — HUME Scheme prelude.
;;;
;;; Evaluated at editor startup before init.scm.  Defines syntax-rules macros
;;; that improve ergonomics over the raw Rust-registered builtins.
;;; Identifiers prefixed `%` are internal Rust forms; user code should call the
;;; unprefixed macro or function instead.
;;;
;;; Macros defined here are visible in init.scm (evaluated globally) and inside
;;; plugin modules loaded via (require).

;; (bind-keys! mode (key cmd) ...)
;;
;; Batched (bind-key!).  Binds one or more key sequences to command names in
;; `mode` ("normal" or "insert").  Expands to a `begin` of individual
;; `bind-key!` calls; all restrictions of `bind-key!` apply.
;;
;; Example:
;;   (bind-keys! "normal"
;;     ("g d" "goto-definition")
;;     ("g r" "goto-references"))
(define-syntax bind-keys!
  (syntax-rules ()
    ((_ mode (key cmd) ...)
     (begin (bind-key! mode key cmd) ...))))

;; (bind-keys-extend! mode (key cmd) ...)
;;
;; Like bind-keys! but each leaf is a force-extend binding.  Equivalent to
;; calling (bind-key-extend! mode key cmd) for each pair.
;;
;; Example:
;;   (bind-keys-extend! "normal"
;;     ("z" "select-line")
;;     ("Z" "select-to-end"))
(define-syntax bind-keys-extend!
  (syntax-rules ()
    ((_ mode (key cmd) ...)
     (begin (bind-key-extend! mode key cmd) ...))))

;; (unbind-keys! mode key ...)
;;
;; Batched (unbind-key!).  Removes one or more key bindings from `mode`.
;; A no-op for keys that are not currently bound.
;;
;; Example:
;;   (unbind-keys! "normal" "Q" "Z Z")
(define-syntax unbind-keys!
  (syntax-rules ()
    ((_ mode key ...)
     (begin (unbind-key! mode key) ...))))
