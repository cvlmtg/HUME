;;; core:lsp/goto.scm — goto family and references. See docs/features.md.

(require "helpers.scm")
(require "locations.scm")

;; Shared by all four goto-family methods and `lsp/references` below. `shape`
;; maps position params to the request's params. See docs/features.md, "Goto
;; and references".
(define (lsp/goto-request pane method
                          #:shape [shape (lambda (params) params)]
                          #:always-drawer? [always-drawer? #f]
                          #:what [what "goto"]
                          #:not-found-msg [not-found-msg "No definition found"]
                          #:require-focus [require-focus #f])
  (lsp/with-servers pane what
    (lambda (servers)
      (let ((tracked (track-position! pane)))
        (lsp-request-all! pane method (shape (lsp-position-params pane))
          (lsp/on-answers what
            (lambda (results)
              (let* ((locations (lsp/answer-locations results))
                     (merged (lsp-locations->merged-places locations))
                     (places (hash-ref merged 'places)))
                (lsp/report-skipped! what (hash-ref merged 'skipped))
                (cond
                  ((null? places) (lsp/report-not-found! locations not-found-msg))
                  ((and (not always-drawer?) (null? (cdr places)))
                   (lsp/jump-to! (hash-ref (car places) 'location)))
                  (else (lsp/show-locations! pane tracked places method shape not-found-msg))))))
          #:require-focus require-focus
          #:tracked tracked)))
    #:method method))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "lsp/goto-definition" "Go to the definition of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/definition")))

(define-command! "lsp/goto-declaration" "Go to the declaration of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/declaration")))

(define-command! "lsp/goto-type-definition" "Go to the type definition of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/typeDefinition")))

(define-command! "lsp/goto-implementation" "Go to the implementation of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/implementation")))

;; ── References ───────────────────────────────────────────────────────────

(define (lsp/references-params params)
  (hash-insert params "context" (hash "includeDeclaration" #t)))

(define-command! "lsp/references" "List references to the symbol under the cursor."
  (lambda (pane)
    (lsp/goto-request pane "textDocument/references"
                      #:shape lsp/references-params
                      #:always-drawer? #t
                      #:what "references"
                      #:not-found-msg "No references found"
                      #:require-focus #t)))
