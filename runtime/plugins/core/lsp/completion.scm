;;; core:lsp/completion.scm — F3: textDocument/completion.
;;;
;;; Stale responses are auto-cancelled/dropped by C6 — never pass
;;; #:allow-stale here (unlike F1's hover).

(require "lib.scm")

;; ── Snippet stripping ────────────────────────────────────────────────────────
;; v1 has no snippet-expansion UI (no tabstop cycling) — inserting raw
;; `${1:foo}`/`$1` syntax verbatim would show it literally in the buffer, so
;; insertTextFormat == 2 (Snippet) items get rewritten to plain text before
;; completion-begin!. Only `${n:default}` -> "default" and bare `$n` -> ""
;; are handled — no choices (`${n|a,b|}`), no nested placeholders, no `\$`
;; escapes. Confirmed acceptable at the manual rust-analyzer smoke (hub's
;; snippet-OQ decision).

(define (lsp/digit? one-char-string)
  (member one-char-string '("0" "1" "2" "3" "4" "5" "6" "7" "8" "9")))

;;; First index of `ch` (a 1-char string) in `text` at or after `start`, or
;;; `#f` if not found.
(define (lsp/find-char text ch start)
  (let ((len (string-length text)))
    (let loop ((i start))
      (cond
        ((>= i len) #f)
        ((equal? (substring text i (+ i 1)) ch) i)
        (else (loop (+ i 1)))))))

(define (lsp/strip-snippet text)
  (let ((len (string-length text)))
    (let loop ((i 0) (out ""))
      (cond
        ((>= i len) out)
        ((and (equal? (substring text i (+ i 1)) "$")
              (< (+ i 1) len)
              (equal? (substring text (+ i 1) (+ i 2)) "{"))
         (let* ((close (lsp/find-char text "}" (+ i 2)))
                (body (substring text (+ i 2) (if close close len)))
                (colon (lsp/find-char body ":" 0))
                (default (if colon (substring body (+ colon 1) (string-length body)) "")))
           (loop (if close (+ close 1) len) (string-append out default))))
        ((and (equal? (substring text i (+ i 1)) "$")
              (< (+ i 1) len)
              (lsp/digit? (substring text (+ i 1) (+ i 2))))
         (let find-end ((j (+ i 1)))
           (if (and (< j len) (lsp/digit? (substring text j (+ j 1))))
               (find-end (+ j 1))
               (loop j out))))
        (else (loop (+ i 1) (string-append out (substring text i (+ i 1)))))))))

;;; Rewrites `insertText`/`textEdit.newText` to plain text when
;;; `insertTextFormat` is 2 (Snippet) — a no-op otherwise.
(define (lsp/strip-snippet-item item)
  (if (and (hash-contains? item "insertTextFormat") (equal? (hash-ref item "insertTextFormat") 2))
      (let* ((item (if (hash-contains? item "insertText")
                        (hash-insert item "insertText" (lsp/strip-snippet (hash-ref item "insertText")))
                        item))
             (item (if (hash-contains? item "textEdit")
                       (let* ((te (hash-ref item "textEdit"))
                              (te (hash-insert te "newText" (lsp/strip-snippet (hash-ref te "newText")))))
                         (hash-insert item "textEdit" te))
                       item)))
        item)
      item))

;; ── Response decoding ───────────────────────────────────────────────────────

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
            (completion-begin! bid (map lsp/strip-snippet-item items) #:incomplete incomplete)))))))

;; ── Trigger entry points ─────────────────────────────────────────────────────
;; Two entry points reach the same request: Ctrl+Space (already bound to
;; this exact command name) and a registered server trigger character.

(define-command! "completion-trigger" "Trigger LSP completion at the cursor."
  (lambda ()
    (lsp/guard-capability "completionProvider"
      (lambda () (lsp/request-and-begin-completions (current-buffer))))))

;;; The char set this feature reacts to — `on-trigger-char` fires for
;;; *every* registered source's chars (union semantics, B10a).
(define *completion-chars* '())

(register-hook! 'on-lsp-attach
  (lambda (bid server-name)
    (let ((caps (lsp-capabilities server-name)))
      (when (and caps (hash-contains? caps "completionProvider"))
        (let* ((cp (hash-ref caps "completionProvider"))
               (triggers (if (hash-contains? cp "triggerCharacters") (hash-ref cp "triggerCharacters") (list))))
          (set! *completion-chars* triggers)
          (register-trigger-chars! "lsp-completion" triggers))))))

;;; `register-trigger-chars!` has no scoping narrower than the source name
;;; ("lsp-completion") — it's global, not per-buffer/per-server, same as
;;; `*completion-chars*` above. Clearing it here on detach is the same
;;; "last call wins" semantics `on-lsp-attach` already has (a second
;;; still-running server sharing this source would have its chars clobbered
;;; the same way a second attach would clobber the first) — not a new
;;; limitation, just symmetric with the existing one.
(register-hook! 'on-lsp-detach
  (lambda (bid server-name)
    (set! *completion-chars* '())
    (register-trigger-chars! "lsp-completion" '())))

(register-hook! 'on-trigger-char
  (lambda (bid ch)
    (when (member ch *completion-chars*)
      (lsp/guard-capability "completionProvider"
        (lambda () (lsp/request-and-begin-completions bid))))))

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

(define (lsp/apply-additional-edits! bid item)
  (when (hash-contains? item "additionalTextEdits")
    (apply-text-edits! bid (map lsp/text-edit->tuple (hash-ref item "additionalTextEdits")))))

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
