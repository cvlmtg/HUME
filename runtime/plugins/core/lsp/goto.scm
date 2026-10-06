;;; core:lsp/goto.scm — goto family and references. See docs/features.md.

(require "lib.scm")
(require "locations.scm")

;; ── Response handling ────────────────────────────────────────────────────────
;; Shared by all four goto-family methods and `lsp-references` below.

;; Every server's locations merge into one list, rows naming the same place
;; once, so two servers agreeing on a definition still jump straight to it.
(define (lsp/goto-response err results pane tracked method
                           #:shape [shape (lambda (params) params)]
                           #:always-drawer? [always-drawer? #f]
                           #:what [what "goto"]
                           #:not-found-msg [not-found-msg "No definition found"])
  (if err
      (lsp/report-error! what err)
      (let ((parts (lsp-locations->display-parts (lsp/answer-locations what results))))
        (cond
          ((null? parts) (log! 'info not-found-msg))
          ((and (not always-drawer?) (null? (cdr parts)))
           (goto-location! (focused-pane) (hash-ref (car parts) 'location)))
          (else (lsp/show-locations! pane tracked parts method shape not-found-msg))))))

(define (lsp/goto-request pane method)
  (lsp/with-position-params pane "goto"
    (lambda (params)
      (let ((tracked (track-position! pane)))
        (lsp-request-all! pane method params
          (lambda (err results) (lsp/goto-response err results pane tracked method))
          #:tracked tracked)))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "lsp-goto-definition" "Go to the definition of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/definition")))

(define-command! "lsp-goto-declaration" "Go to the declaration of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/declaration")))

(define-command! "lsp-goto-type-definition" "Go to the type definition of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/typeDefinition")))

(define-command! "lsp-goto-implementation" "Go to the implementation of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/implementation")))

;; ── References ───────────────────────────────────────────────────────────

(define (lsp/references-params params)
  (hash-insert params "context" (hash "includeDeclaration" #t)))

(define-command! "lsp-references" "List references to the symbol under the cursor."
  (lambda (pane)
    (lsp/with-position-params pane "references"
      (lambda (params)
        (let ((tracked (track-position! pane)))
          (lsp-request-all! pane "textDocument/references"
            (lsp/references-params params)
            (lambda (err results)
              (lsp/goto-response err results pane tracked "textDocument/references"
                                 #:shape lsp/references-params
                                 #:always-drawer? #t
                                 #:what "references"
                                 #:not-found-msg "No references found"))
            #:require-focus #t
            #:tracked tracked))))))
