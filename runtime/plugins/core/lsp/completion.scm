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

;; ── The source ───────────────────────────────────────────────────────────────

(register-completion-source! "lsp"
  (lambda (id bid prefix)
    (if (lsp/supports-for-buffer? bid "completionProvider")
        (lsp-request #f "textDocument/completion" (lsp-position-params bid)
          (lambda (err res)
            (cond
              (err (lsp/report-error "completion" err)
                   (completion-emit! id '()))
              ((void? res) (completion-emit! id '()))
              (else
                (let ((decoded (lsp/completion-response->items res)))
                  (completion-emit! id (car decoded) #:incomplete (cadr decoded))))))
          #:supersede "completion")
        (completion-emit! id '())))
  #:target 'buffer #:priority 10)

;; ── Trigger chars ─────────────────────────────────────────────────────────────

(lsp/setup-trigger-chars! "completionProvider" "lsp" '() #f)

;; ── Accept ────────────────────────────────────────────────────────────────────
;; No `on-completion-accept` handler here, deliberately — see docs/features.md.
