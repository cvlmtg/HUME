;;; core:lsp/actions.scm — textDocument/codeAction. See docs/features.md.

(require "lib.scm")

(define (lsp/primary-selection-range pane)
  (let ((primary (call! "stdlib/primary-selection" (buffer-selections pane))))
    (and primary
         (let ((a (call! "stdlib/selection-anchor" primary))
               (h (call! "stdlib/selection-head" primary)))
           (cons (min a h) (+ (max a h) 1))))))

(define (lsp/action-disabled? action)
  (not (equal? (json-ref-or action #f "disabled") #f)))

(define (lsp/action-title action)
  (json-ref action "title"))

(define (lsp/action-resolve-provider? pane)
  (lsp/cap-flag? pane "codeActionProvider" "resolveProvider"))

;;; `pane` is the buffer the action came from — captured by
;;; `"lsp-code-actions"` when the request was sent, threaded through the
;;; menu selection and (for an unresolved action) the `codeAction/resolve`
;;; round trip, never re-read from focus. Both round trips are async (the
;;; user picks a menu item, then waits on the network), so this is the same
;;; capture-at-source discipline every other chained LSP request here uses.
;;; `gen` is that same buffer's generation at the same capture point,
;;; threaded alongside `pane` for the same reason — `apply-workspace-edit!`
;;; checks it before applying, so an edit computed against text that has
;;; since changed fails loudly instead of applying against the wrong text.
;;; `#:allow-stale #t`: `workspace/executeCommand`'s params carry no
;;; `textDocument`, so the bridge's own text-gen anchor has nothing buffer-
;;; specific to check against `pane` — without this, the anchor's fallback
;;; (its own current generation vs. `pane`'s at drain time) would drop the
;;; response on any intervening edit. Safe to skip: this callback only
;;; reports an error; the command's actual edits (if any) arrive separately
;;; via a server-initiated `workspace/applyEdit`, which carries its own
;;; positions and is never subject to this staleness check.
(define (lsp/exec-command pane cmd-obj)
  (lsp-request pane "workspace/executeCommand"
    (hash "command" (json-ref cmd-obj "command")
          "arguments" (json-ref-or cmd-obj (list) "arguments"))
    (lambda (err res) (when err (lsp/report-error "code action" err)))
    #:allow-stale #t))

(define (lsp/run-action pane action gen #:resolved? [resolved? #f])
  (let ((edit (json-ref-or action #f "edit"))
        (command (json-ref-or action #f "command")))
    (cond
      ((or edit command)
       (when edit (apply-workspace-edit! pane edit #:expect-generation gen))
       (when command (lsp/exec-command pane (if (string? command) action command))))
      ((and (not resolved?) (lsp/action-resolve-provider? pane))
       ;; `#:allow-stale #t`: unlike the menu-building request above, this
       ;; response *is* the edit — dropping it on an intervening keystroke
       ;; (the previous, gen-checked behavior) would silently do nothing
       ;; after the user already picked an action from the menu. Safe to
       ;; deliver stale: `apply-workspace-edit!`'s own `#:expect-generation`
       ;; check below still fails loudly if the buffer actually changed.
       (lsp-request pane "codeAction/resolve" action
         (lambda (err resolved)
           (cond
             (err (lsp/report-error "code action" err))
             ((void? resolved) (log! 'info "Code action has no edit or command"))
             (else (lsp/run-action pane resolved gen #:resolved? #t))))
         #:allow-stale #t))
      (else (log! 'info "Code action has no edit or command")))))

(define-command! "lsp-code-actions" "Show available code actions for the cursor or selection."
  (lambda (pane)
    (lsp/guard-capability pane "codeActionProvider"
      (lambda ()
        (let* ((gen (buffer-generation pane))
               (diags (diagnostics-for-buffer pane #:range (lsp/primary-selection-range pane)))
               (context (hash "diagnostics" (map (lambda (d) (hash-ref d "raw")) diags)
                              "triggerKind" 1)))
          (lsp-request pane "textDocument/codeAction"
            (hash-insert (lsp-primary-range-params pane) "context" context)
            (lambda (err res)
              (cond
                (err (lsp/report-error "code action" err))
                ((void? res) (log! 'info "No code actions"))
                (else
                  (let ((actions (filter (lambda (a) (not (lsp/action-disabled? a))) (json-list res))))
                    (if (null? actions)
                        (log! 'info "No code actions")
                        (show-menu! pane (map lsp/action-title actions)
                          (lambda (idx) (when idx (lsp/run-action pane (list-ref actions idx) gen)))))))))
            #:require-focus #t))))))
