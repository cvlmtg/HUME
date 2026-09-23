;;; core:lsp/actions.scm — textDocument/codeAction. See docs/features.md.

(require "lib.scm")

(define (lsp/primary-selection-range)
  (let ((primary (call! "stdlib/primary-selection" (current-selections))))
    (and primary
         (let ((a (call! "stdlib/selection-anchor" primary))
               (h (call! "stdlib/selection-head" primary)))
           (cons (min a h) (+ (max a h) 1))))))

(define (lsp/action-disabled? action)
  (and (json-contains? action "disabled")
       (not (equal? (json-ref action "disabled") #f))))

(define (lsp/action-title action)
  (json-ref action "title"))

(define (lsp/action-resolve-provider?)
  (lsp/cap-flag? "codeActionProvider" "resolveProvider"))

(define (lsp/exec-command cmd-obj)
  (lsp-request #f "workspace/executeCommand"
    (hash "command" (json-ref cmd-obj "command")
          "arguments" (if (json-contains? cmd-obj "arguments")
                           (json-ref cmd-obj "arguments")
                           (list)))
    (lambda (err res) (when err (lsp/report-error "code action" err)))))

(define (lsp/run-action action #:resolved? [resolved? #f])
  (cond
    ((or (json-contains? action "edit") (json-contains? action "command"))
     (when (json-contains? action "edit")
       (apply-workspace-edit! (json-ref action "edit")))
     (when (json-contains? action "command")
       (let ((cmd (json-ref action "command")))
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
                  ((void? res) (log! 'info "No code actions"))
                  (else
                    (let ((actions (filter (lambda (a) (not (lsp/action-disabled? a))) (json-list res))))
                      (if (null? actions)
                          (log! 'info "No code actions")
                          (show-menu! (map lsp/action-title actions)
                            (lambda (idx) (when idx (lsp/run-action (list-ref actions idx)))))))))))))))))
