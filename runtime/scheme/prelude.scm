;;; runtime/scheme/prelude.scm — HUME Scheme prelude.
;;; What this is and why call! isn't here: prelude.md, this directory.

(define-syntax bind-keys!
  (syntax-rules ()
    ((_ mode #:keymap keymap (key cmd) ...)
     (begin (bind-key! mode key cmd #:keymap keymap) ...))
    ((_ mode (key cmd) ...)
     (begin (bind-key! mode key cmd) ...))))

(define-syntax bind-keys-extend!
  (syntax-rules ()
    ((_ mode #:keymap keymap (key cmd) ...)
     (begin (bind-key-extend! mode key cmd #:keymap keymap) ...))
    ((_ mode (key cmd) ...)
     (begin (bind-key-extend! mode key cmd) ...))))

(define-syntax unbind-keys!
  (syntax-rules ()
    ((_ mode #:keymap keymap key ...)
     (begin (unbind-key! mode key #:keymap keymap) ...))
    ((_ mode key ...)
     (begin (unbind-key! mode key) ...))))

(define (define-language! name [exts '()] [globs '()] [shebangs '()])
  (%define-language! name exts globs shebangs))

(define (configure-language! name
                             #:roots [roots #f]
                             #:language-id [language-id #f]
                             #:comment-tokens [comment-tokens #f]
                             #:block-comment-tokens [block-comment-tokens #f])
  (%configure-language! name (hash 'roots roots
                                   'language-id language-id
                                   'comment-tokens comment-tokens
                                   'block-comment-tokens block-comment-tokens)))

(define (register-grammar! name grammar-path symbol highlights-path
                           #:injections [injections-path #f]
                           #:textobjects [textobjects-path #f]
                           #:indents [indents-path #f])
  (%register-grammar! name grammar-path symbol highlights-path
                      (hash 'injections injections-path
                            'textobjects textobjects-path
                            'indents indents-path)))

(define (configure-statusline! left center right)
  (set-option! "statusline" (hash 'left left 'center center 'right right)))

(define-command! "insert-register"
  "Insert the register named by the next key, or, for f, the buffer's file path and, for p, its directory, relative to the working directory."
  (lambda (pane)
    (on-next-char!
      (lambda (ch)
        (let ([text
               (cond
                 [(not ch) #f]
                 [(eqv? ch #\f) (buffer-display-path pane #:relative #t)]
                 [(eqv? ch #\p)
                  (let ([file (buffer-display-path pane #:relative #t)])
                    (and file
                         (let ([dir (parent-name file)])
                           (path-join (if (equal? dir "") "." dir) ""))))]
                 [(register-name? (string ch)) (read-register (string ch))]
                 [else
                  (error (string-append "insert-register: invalid register '" (string ch) "'"))])])
          (when text (paste-text! pane text))))))
  #:repeatable #t)

(bind-key! 'insert "ctrl-r" "insert-register")
(bind-key! 'command "ctrl-r" "insert-register")
