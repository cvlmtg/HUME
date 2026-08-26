;;; core:lsp/completion.scm — textDocument/completion. See README.md "How it
;;; works" → "Completion". Stale responses are auto-cancelled/dropped —
;;; never pass #:allow-stale here (unlike hover).

(require "lib.scm")

;; ── Response decoding ───────────────────────────────────────────────────────
;; Snippet stripping happens in Rust at the store ingress — items arriving
;; here already have plain `insertText`/`textEdit.newText`.

;;; `res`: a bare `CompletionItem[]` (incomplete implicitly `#f`) or a
;;; `CompletionList` hashmap `{isIncomplete, items}`. Returns `(list items
;;; incomplete)`.
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
    ;; Per-keystroke refiltering can re-issue this before the prior response
    ;; lands — supersede rather than race two sessions.
    #:supersede "completion"))

;; ── Trigger entry points ─────────────────────────────────────────────────────
;; Two entry points reach the same request: Ctrl+Space (bound to this exact
;; command name in plugin.scm) and a registered server trigger character.

(define-command! "lsp-completion-trigger" "Trigger LSP completion at the cursor."
  (lambda ()
    (lsp/guard-capability "completionProvider"
      (lambda () (lsp/request-and-begin-completions (current-buffer))))))

(lsp/setup-trigger-chars! "completionProvider" "lsp-completion" '()
  (lambda (bid ch)
    (lsp/guard-capability "completionProvider"
      (lambda () (lsp/request-and-begin-completions bid)))))

;; ── isIncomplete re-request ──────────────────────────────────────────────────
;; Fires only while the open session's isIncomplete flag is set — capability
;; was already confirmed to start that session, no re-guard here.

(register-hook! 'on-completion-refilter
  (lambda (bid filter-text)
    (lsp/request-and-begin-completions bid)))

;; ── Accept ────────────────────────────────────────────────────────────────────
;; No `on-completion-accept` handler here, deliberately — see README.

