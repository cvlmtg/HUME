;;; core:lsp/actions.scm — textDocument/codeAction. See docs/features.md.

(require "lib.scm")

;;; `(start . end)`, end exclusive, or `#f` — see docs/features.md.
(define (lsp/primary-selection-range)
  (let ((primary (call! "stdlib/primary-selection" (current-selections))))
    (and primary
         (let ((a (call! "stdlib/selection-anchor" primary))
               (h (call! "stdlib/selection-head" primary)))
           (cons (min a h) (+ (max a h) 1))))))

;;; LSP 3.16 `{"reason": string}` — never shown in the menu.
(define (lsp/action-disabled? action)
  (and (hash-contains? action "disabled")
       (not (equal? (hash-ref action "disabled") #f))))

(define (lsp/action-title action)
  (hash-ref action "title"))

(define (lsp/action-resolve-provider?)
  (lsp/cap-flag? "codeActionProvider" "resolveProvider"))

;;; `cmd-obj`: a Command `{title, command, arguments?}`.
(define (lsp/exec-command cmd-obj)
  (lsp-request #f "workspace/executeCommand"
    (hash "command" (hash-ref cmd-obj "command")
          "arguments" (if (hash-contains? cmd-obj "arguments")
                           (hash-ref cmd-obj "arguments")
                           (list)))
    (lambda (err res) (when err (lsp/report-error "code action" err)))))

;;; See docs/features.md for the edit-then-command ordering and the
;;; resolve-fallback round trip.
(define (lsp/run-action action #:resolved? [resolved? #f])
  (cond
    ((or (hash-contains? action "edit") (hash-contains? action "command"))
     (when (hash-contains? action "edit")
       (apply-workspace-edit! (hash-ref action "edit")))
     (when (hash-contains? action "command")
       (let ((cmd (hash-ref action "command")))
         ;; Bare legacy `Command`: `command` is a string at the top level,
         ;; no `edit` key — `action` itself is then the Command object.
         (lsp/exec-command (if (string? cmd) action cmd)))))
    ((and (not resolved?) (lsp/action-resolve-provider?))
     (lsp-request #f "codeAction/resolve" action
       (lambda (err resolved)
         (cond
           (err (lsp/report-error "code action" err))
           ((void? resolved) (log! 'info "Code action has no edit or command"))
           (else (lsp/run-action resolved #:resolved? #t))))))
    (else (log! 'info "Code action has no edit or command"))))

(define-command! "lsp-code-actions" "Show available code actions for the cursor or selection."
  (lambda ()
    (let ((bid (current-buffer)))
      (lsp/guard-capability "codeActionProvider"
        (lambda ()
          (let* ((diags (diagnostics-for-buffer bid #:range (lsp/primary-selection-range)))
                 (context (hash "diagnostics" (map (lambda (d) (hash-ref d "raw")) diags)
                                "triggerKind" 1)))
            (lsp-request #f "textDocument/codeAction"
              (hash-insert (lsp-primary-range-params bid) "context" context)
              (lambda (err res)
                (cond
                  (err (lsp/report-error "code action" err))
                  ((or (void? res) (null? res)) (log! 'info "No code actions"))
                  (else
                    (let ((actions (filter (lambda (a) (not (lsp/action-disabled? a))) res)))
                      (if (null? actions)
                          (log! 'info "No code actions")
                          (show-menu! (map lsp/action-title actions)
                            (lambda (idx) (when idx (lsp/run-action (list-ref actions idx)))))))))))))))))
