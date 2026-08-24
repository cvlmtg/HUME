;;; core:lsp/goto.scm — goto definition family; references (reuses
;;; the same worker with context.includeDeclaration added).

(require "lib.scm")

;; ── Response handling ────────────────────────────────────────────────────────
;; Shared by all four goto-family methods and `lsp-references` below:
;; err -> report it; null/empty -> "no results"; a single Location hashmap ->
;; jump directly; a Location[]/LocationLink[] array -> jump if it has exactly
;; one entry (unless `always-drawer?`), else list them in the drawer.

(define (lsp/goto-response err res #:always-drawer? [always-drawer? #f]
                                    #:what [what "goto"]
                                    #:not-found-msg [not-found-msg "No definition found"])
  (cond
    (err (lsp/report-error what err))
    ((void? res) (log! 'info not-found-msg))
    ((list? res)
     (cond
       ((null? res) (log! 'info not-found-msg))
       ((and (not always-drawer?) (= (length res) 1)) (goto-location! (car res)))
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

;; ── References ───────────────────────────────────────────────────────────
;; Always the drawer, even for one result — "where is this used" expects a
;; list, unlike goto's "take me there". Delegates to `lsp/goto-response`'s
;; err/void/null cascade rather than re-implementing it — the bare-Location
;; `else` branch it also carries is simply unreached here:
;; `textDocument/references` only ever returns `Location[] | null` per spec,
;; never a bare `Location`.

(define-command! "lsp-references" "List references to the symbol under the cursor."
  (lambda ()
    (lsp/guard-capability "referencesProvider"
      (lambda ()
        (lsp-request #f "textDocument/references"
          (hash-insert (lsp-position-params (current-buffer))
                       "context" (hash "includeDeclaration" #t))
          (lambda (err res)
            (lsp/goto-response err res #:always-drawer? #t
                                       #:what "references"
                                       #:not-found-msg "No references found")))))))
