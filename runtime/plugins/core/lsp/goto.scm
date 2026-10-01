;;; core:lsp/goto.scm — goto family and references. See docs/features.md.

(require "lib.scm")
(require "locations.scm")

;; ── Response handling ────────────────────────────────────────────────────────
;; Shared by all four goto-family methods and `lsp-references` below.

(define (lsp/goto-response err res pane tracked method shape
                           #:always-drawer? [always-drawer? #f]
                           #:what [what "goto"]
                           #:not-found-msg [not-found-msg "No definition found"])
  (let ((locs (if err '() (lsp/response-locations res))))
    (cond
      (err (lsp/report-error! what err))
      ((null? locs) (log! 'info not-found-msg))
      ((and (not always-drawer?) (null? (cdr locs)))
       (goto-location! (focused-pane) (car locs)))
      (else (lsp/show-locations! pane tracked locs method shape not-found-msg)))))

(define (lsp/same-params params) params)

(define (lsp/goto-request pane method cap)
  (lsp/guard-capability pane cap
    (lambda ()
      (let ((tracked (track-position! pane)))
        (lsp-request! pane method (lsp-position-params pane)
          (lambda (err res) (lsp/goto-response err res pane tracked method lsp/same-params))
          #:tracked tracked)))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "lsp-goto-definition" "Go to the definition of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/definition" "definitionProvider")))

(define-command! "lsp-goto-declaration" "Go to the declaration of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/declaration" "declarationProvider")))

(define-command! "lsp-goto-type-definition" "Go to the type definition of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/typeDefinition" "typeDefinitionProvider")))

(define-command! "lsp-goto-implementation" "Go to the implementation of the symbol under the cursor."
  (lambda (pane) (lsp/goto-request pane "textDocument/implementation" "implementationProvider")))

;; ── References ───────────────────────────────────────────────────────────

(define (lsp/references-params params)
  (hash-insert params "context" (hash "includeDeclaration" #t)))

(define-command! "lsp-references" "List references to the symbol under the cursor."
  (lambda (pane)
    (lsp/guard-capability pane "referencesProvider"
      (lambda ()
        (let ((tracked (track-position! pane)))
          (lsp-request! pane "textDocument/references"
            (lsp/references-params (lsp-position-params pane))
            (lambda (err res)
              (lsp/goto-response err res pane tracked "textDocument/references" lsp/references-params
                                 #:always-drawer? #t
                                 #:what "references"
                                 #:not-found-msg "No references found"))
            #:require-focus #t
            #:tracked tracked))))))
