;;; runtime/scheme/prelude.scm — HUME Scheme prelude.
;;;
;;; Evaluated at editor startup before init.scm.  Defines syntax-rules macros
;;; that improve ergonomics over the raw Rust-registered builtins.
;;; Identifiers prefixed `%` are internal Rust forms; user code should call the
;;; unprefixed macro or function instead.
;;;
;;; Macros defined here are visible in init.scm (evaluated globally) and inside
;;; plugin modules loaded via (require).
;;;
;;; NOTE: (call! name args…) is NOT defined here.  It is a core dispatch
;;; primitive defined in builtins/bootstrap.scm (embedded via include_str! in
;;; builtins/mod.rs) so it is unconditionally available — even in test
;;; engines that never load the prelude.  The prelude is optional (silent
;;; no-op if the runtime dir is missing); call! must not be.

;; (bind-keys! mode (key cmd) ...)
;;
;; Batched (bind-key!).  Binds one or more key sequences to command names in
;; `mode` ('normal, 'extend, or 'insert).  Expands to a `begin` of individual
;; `bind-key!` calls; all restrictions of `bind-key!` apply.
;;
;; Example:
;;   (bind-keys! 'normal
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
;;   (bind-keys-extend! 'normal
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
;;   (unbind-keys! 'normal "Q" "Z Z")
(define-syntax unbind-keys!
  (syntax-rules ()
    ((_ mode key ...)
     (begin (unbind-key! mode key) ...))))

;; (define-language! name [exts [globs [shebangs]]])
;;
;; Register a language identity.  Trailing args default to empty lists so
;; callers only need to supply what they use.  Delegates to %define-language!
;; (a Rust builtin, init-only).
;;
;; Example:
;;   (define-language! "markdown" '("md" "mkd"))
;;   (define-language! "makefile" '() '("Makefile" "GNUmakefile"))
(define-syntax define-language!
  (syntax-rules ()
    ((_ name)
     (%define-language! name '() '() '()))
    ((_ name exts)
     (%define-language! name exts '() '()))
    ((_ name exts globs)
     (%define-language! name exts globs '()))
    ((_ name exts globs shebangs)
     (%define-language! name exts globs shebangs))))

;; (register-grammar! name grammar-path symbol highlights-path [injections-path])
;;
;; Attach a tree-sitter grammar to a language. `injections-path` defaults to
;; `#f` (no embedded-language support) when omitted. Delegates to
;; %register-grammar! (a Rust builtin).
(define-syntax register-grammar!
  (syntax-rules ()
    ((_ name grammar-path symbol highlights-path)
     (%register-grammar! name grammar-path symbol highlights-path #f))
    ((_ name grammar-path symbol highlights-path injections-path)
     (%register-grammar! name grammar-path symbol highlights-path injections-path))))
