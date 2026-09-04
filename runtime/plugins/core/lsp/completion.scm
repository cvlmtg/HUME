;;; core:lsp/completion.scm — textDocument/completion. See docs/features.md.
;;; Never pass #:allow-stale here (unlike hover) — stale responses are
;;; auto-cancelled/dropped.

(require "lib.scm")

;; ── Response decoding ───────────────────────────────────────────────────────

;;; `res`: a bare `CompletionItem[]` (incomplete implicitly `#f`) or a
;;; `CompletionList` hashmap `{isIncomplete, items}`.
(define (lsp/completion-response->items res)
  (if (list? res)
      (list res #f)
      (list (hash-ref res "items")
            (if (hash-contains? res "isIncomplete") (hash-ref res "isIncomplete") #f))))

;; ── Request + begin ──────────────────────────────────────────────────────────

(define (lsp/request-and-begin-completions bid)
  (lsp-request #f "textDocument/completion" (lsp-position-params bid)
    (lambda (err res)
      (cond
        (err (lsp/report-error "completion" err))
        ((void? res) (begin)) ; null -> no session
        (else
          (let* ((decoded (lsp/completion-response->items res))
                 (items (car decoded))
                 (incomplete (cadr decoded)))
            (completion-begin! bid items #:incomplete incomplete)))))
    #:supersede "completion"))

;; ── Trigger entry points ─────────────────────────────────────────────────────

(define-command! "lsp-completion-trigger" "Trigger LSP completion at the cursor."
  (lambda ()
    (lsp/guard-capability "completionProvider"
      (lambda () (lsp/request-and-begin-completions (current-buffer))))))

(lsp/setup-trigger-chars! "completionProvider" "lsp-completion" '()
  (lambda (bid ch)
    (lsp/guard-capability "completionProvider"
      (lambda () (lsp/request-and-begin-completions bid)))))

;; ── isIncomplete re-request ──────────────────────────────────────────────────

(register-hook! 'on-completion-refilter
  (lambda (bid filter-text)
    (lsp/request-and-begin-completions bid)))

;; ── Accept ────────────────────────────────────────────────────────────────────
;; No `on-completion-accept` handler here, deliberately — see docs/features.md.

