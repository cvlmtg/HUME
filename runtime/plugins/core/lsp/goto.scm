;;; core:lsp/goto.scm — F2: goto definition family; F6: references (reuses
;;; the same worker with context.includeDeclaration added).

(require "lib.scm")

;; ── Response handling ────────────────────────────────────────────────────────
;; Shared by all four goto-family methods (and F6's references, always-drawer
;; variant below): null/empty -> "no results"; a single Location hashmap ->
;; jump directly; a Location[]/LocationLink[] array -> jump if it has exactly
;; one entry, else list them in the drawer.

(define (lsp/goto-response err res)
  (cond
    (err (lsp/report-error "goto" err))
    ((void? res) (log! 'info "No definition found"))
    ((list? res)
     (cond
       ((null? res) (log! 'info "No definition found"))
       ((= (length res) 1) (goto-location! (car res)))
       (else (lsp/show-locations! res))))
    (else (goto-location! res))))

(define (lsp/goto-request method cap)
  (lsp/guard-capability cap
    (lambda ()
      (lsp-request #f method (lsp-position-params (current-buffer)) lsp/goto-response))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "lsp-goto-definition" "Go to the definition of the symbol under the cursor."
  (lambda () (lsp/goto-request "textDocument/definition" "definitionProvider")))

(define-command! "lsp-goto-declaration" "Go to the declaration of the symbol under the cursor."
  (lambda () (lsp/goto-request "textDocument/declaration" "declarationProvider")))

(define-command! "lsp-goto-type-definition" "Go to the type definition of the symbol under the cursor."
  (lambda () (lsp/goto-request "textDocument/typeDefinition" "typeDefinitionProvider")))

(define-command! "lsp-goto-implementation" "Go to the implementation of the symbol under the cursor."
  (lambda () (lsp/goto-request "textDocument/implementation" "implementationProvider")))
