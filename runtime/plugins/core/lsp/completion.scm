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

;; ── Post-accept: additionalTextEdits, resolve ────────────────────────────────
;; Rust applies only the item's main edit (textEdit or insertText) before
;; firing this — auto-import edits and anything only available via resolve
;; are Steel's job from here.

(define (lsp/completion-resolve-provider?)
  (let ((caps (lsp-capabilities #f)))
    (and caps
         (hash-contains? caps "completionProvider")
         (let ((cp (hash-ref caps "completionProvider")))
           (and (hash-contains? cp "resolveProvider")
                (equal? (hash-ref cp "resolveProvider") #t))))))

;;; Rust already applied `item`'s main edit before this fires — an
;;; `additionalTextEdits` entry on the *same line* as the main edit's end,
;;; at or after its end column, still carries its pre-edit column and now
;;; lands short/long by the main edit's UTF-16 length delta.
;;; `end-line`/`end-char`/`delta` describe the main edit's already-applied
;;; end position and length change; a different-line edit (the common
;;; case — a top-of-file auto-import) is untouched. additionalTextEdits
;;; never overlap the main edit (LSP spec), so one starting before the
;;; main edit's end is entirely unaffected by it, never partially.
(define (lsp/shift-additional-edit end-line end-char delta te)
  (let* ((range (hash-ref te "range"))
         (start (hash-ref range "start"))
         (end (hash-ref range "end")))
    (if (and (equal? (hash-ref start "line") end-line)
             (>= (hash-ref start "character") end-char))
        (hash-insert te "range"
          (hash "start" (hash-insert start "character" (+ (hash-ref start "character") delta))
                "end" (hash-insert end "character" (+ (hash-ref end "character") delta))))
        te)))

(define (lsp/apply-additional-edits! bid item)
  (when (hash-contains? item "additionalTextEdits")
    (let* ((edits (hash-ref item "additionalTextEdits"))
           (edits
             (if (hash-contains? item "textEdit")
                 (let* ((te (hash-ref item "textEdit"))
                        (main-range (hash-ref te "range"))
                        (main-start (hash-ref main-range "start"))
                        (main-end (hash-ref main-range "end"))
                        (delta (- (lsp/string-utf16-length (hash-ref te "newText"))
                                  (- (hash-ref main-end "character") (hash-ref main-start "character")))))
                   (map (lambda (e)
                          (lsp/shift-additional-edit
                            (hash-ref main-end "line") (hash-ref main-end "character") delta e))
                        edits))
                 edits)))
      (apply-text-edits! bid (map lsp/text-edit->tuple edits)))))

(register-hook! 'on-completion-accept
  (lambda (bid item)
    (cond
      ((hash-contains? item "additionalTextEdits") (lsp/apply-additional-edits! bid item))
      ((lsp/completion-resolve-provider?)
       (lsp-request #f "completionItem/resolve" item
         (lambda (err resolved)
           (cond
             (err (lsp/report-error "completion resolve" err))
             ((void? resolved) (begin))
             (else (lsp/apply-additional-edits! bid resolved)))))))))

;; ── isIncomplete re-request ──────────────────────────────────────────────────
;; Rust only fires this while the open session's isIncomplete flag is set —
;; capability was already confirmed to start that session, no re-guard here.

(register-hook! 'on-completion-refilter
  (lambda (bid filter-text)
    (lsp/request-and-begin-completions bid)))
