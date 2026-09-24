;;; core:lsp/goto.scm — goto definition family; references (reuses the same
;;; worker with context.includeDeclaration added). See docs/features.md.

(require "lib.scm")

;; ── Response handling ────────────────────────────────────────────────────────
;; Shared by all four goto-family methods and `lsp-references` below.

;;; `res`'s `JsonHandle` (and every location `json-list` pulls out of it)
;;; carries the producing server's own tagged encoding, so `goto-location!`
;;; and `lsp-locations->display-parts` decode it correctly with no pane
;;; capture needed here at all — see `lsp/show-locations!`'s own doc for
;;; why the jump itself also reads `(focused-pane)` rather than a captured
;;; invocation pane.
(define (lsp/goto-response err res #:always-drawer? [always-drawer? #f]
                                    #:what [what "goto"]
                                    #:not-found-msg [not-found-msg "No definition found"])
  (cond
    (err (lsp/report-error what err))
    ((void? res) (log! 'info not-found-msg))
    ((json-array? res)
     (let ((locs (json-list res)))
       (cond
         ((null? locs) (log! 'info not-found-msg))
         ((and (not always-drawer?) (= (length locs) 1)) (goto-location! (focused-pane) (car locs)))
         (else (lsp/show-locations! locs)))))
    (else (goto-location! (focused-pane) res))))

;;; No `#:require-focus` here, unlike hover/signature-help/code-actions: a
;;; goto request is a navigation the user asked for, not info anchored to
;;; where the cursor happens to be right now — completing the jump once it
;;; resolves is correct even if the user looked elsewhere while waiting,
;;; the same way pressing Enter on a slow-loading link still navigates.
(define (lsp/goto-request pane method cap)
  (lsp/guard-capability pane cap
    (lambda ()
      (lsp-request pane method (lsp-position-params pane)
        (lambda (err res) (lsp/goto-response err res))))))

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

(define-command! "lsp-references" "List references to the symbol under the cursor."
  (lambda (pane)
    (lsp/guard-capability pane "referencesProvider"
      (lambda ()
        (lsp-request pane "textDocument/references"
          (hash-insert (lsp-position-params pane)
                       "context" (hash "includeDeclaration" #t))
          (lambda (err res)
            (lsp/goto-response err res #:always-drawer? #t
                                       #:what "references"
                                       #:not-found-msg "No references found"))
          #:require-focus #t)))))
