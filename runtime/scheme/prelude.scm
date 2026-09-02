;;; runtime/scheme/prelude.scm — HUME Scheme prelude.
;;; What this is and why call! isn't here: prelude.md, this directory.

(define-syntax bind-keys!
  (syntax-rules ()
    ((_ mode (key cmd) ...)
     (begin (bind-key! mode key cmd) ...))))

(define-syntax bind-keys-extend!
  (syntax-rules ()
    ((_ mode (key cmd) ...)
     (begin (bind-key-extend! mode key cmd) ...))))

(define-syntax unbind-keys!
  (syntax-rules ()
    ((_ mode key ...)
     (begin (unbind-key! mode key) ...))))

(define (define-language! name
                          [exts '()] [globs '()] [shebangs '()]
                          #:language-id [language-id #f])
  (%define-language! name exts globs shebangs language-id))

(define-syntax register-grammar!
  (syntax-rules ()
    ((_ name grammar-path symbol highlights-path)
     (%register-grammar! name grammar-path symbol highlights-path #f #f))
    ((_ name grammar-path symbol highlights-path injections-path)
     (%register-grammar! name grammar-path symbol highlights-path injections-path #f))
    ((_ name grammar-path symbol highlights-path injections-path textobjects-path)
     (%register-grammar! name grammar-path symbol highlights-path injections-path textobjects-path))))
