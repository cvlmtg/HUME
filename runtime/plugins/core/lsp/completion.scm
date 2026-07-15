;;; core:lsp/completion.scm — textDocument/completion.
;;;
;;; Stale responses are auto-cancelled/dropped — never pass
;;; #:allow-stale here (unlike hover).

(require "lib.scm")

;; ── Response decoding ───────────────────────────────────────────────────────
;;
;; Snippet stripping (insertTextFormat: Snippet items rewritten to plain
;; text — v1 has no tabstop-cycling UI) happens in Rust, at the store
;; ingress (`StoredCompletionItem::from_typed`/`from_json_lenient`,
;; hume-editor/src/editor/lsp/completion.rs) — items arriving here already
;; have plain `insertText`/`textEdit.newText`.

;;; `res`: a bare `CompletionItem[]` (JSON array -> Steel list, incomplete
;;; implicitly `#f`) or a `CompletionList` hashmap `{isIncomplete, items}`.
;;; Returns `(list items incomplete)`.
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
    ;; Per-keystroke refiltering (`on-completion-refilter`) can re-issue this
    ;; before the prior response lands — cancel it rather than let two
    ;; in-flight completion requests race each other for the same session.
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

;; ── on-completion-accept ─────────────────────────────────────────────────────
;; Rust applies the main edit, any additionalTextEdits, and (when the item
;; lacked additionalTextEdits but the server advertises resolveProvider) the
;; completionItem/resolve round trip — all atomically, through the same
;; ChangeSet the accept edit produced (see `CompletionSession::accept` and
;; `edits::apply_resolved_additional_edits`, hume-editor/src/editor/lsp/).
;; on-completion-accept remains a plain extension point for anything this
;; store doesn't parse (e.g. `command`) — no default handler needed here.

;; ── isIncomplete re-request ──────────────────────────────────────────────────
;; Rust only fires this while the open session's isIncomplete flag is set —
;; capability was already confirmed to start that session, no re-guard here.

(register-hook! 'on-completion-refilter
  (lambda (bid filter-text)
    (lsp/request-and-begin-completions bid)))
