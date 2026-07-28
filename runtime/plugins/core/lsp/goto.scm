;;; core:lsp/goto.scm — goto definition family; references (reuses
;;; the same worker with context.includeDeclaration added).

(require "lib.scm")

;; ── Response handling ────────────────────────────────────────────────────────
;; Shared by all four goto-family methods (and references, always-drawer
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
       ((= (length res) 1) (goto-location! (lsp/normalize-location (car res))))
       (else (lsp/show-locations! (map lsp/normalize-location res)))))
    (else (goto-location! (lsp/normalize-location res)))))

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

;; ── References ───────────────────────────────────────────────────────────
;; Always the drawer, even for one result — "where is this used" expects a
;; list, unlike goto's "take me there".

(define-command! "lsp-references" "List references to the symbol under the cursor."
  (lambda ()
    (lsp/guard-capability "referencesProvider"
      (lambda ()
        (lsp-request #f "textDocument/references"
          (hash-insert (lsp-position-params (current-buffer))
                       "context" (hash "includeDeclaration" #t))
          (lambda (err res)
            (cond
              (err (lsp/report-error "references" err))
              ((void? res) (log! 'info "No references found"))
              ((null? res) (log! 'info "No references found"))
              ;; No separate bare-Location branch here, unlike
              ;; `lsp/goto-response` above: `textDocument/references` only
              ;; ever returns `Location[] | null` per spec, never a bare
              ;; `Location`, so there's no single-hashmap case to guard against.
              (else (lsp/show-locations! (map lsp/normalize-location res))))))))))
