;;; core:lsp/actions.scm — textDocument/codeAction. See docs/features.md.

(require "helpers.scm")

(define (lsp/primary-selection-range pane)
  (let ((primary (call! "stdlib/primary-selection" (buffer-selections pane))))
    (and primary
         (hash 'start-char (selection-start-char primary)
               'end-char (selection-end-char primary)))))

(define lsp/no-edit-message "Code action has no edit or command")

;; `server` produced the action: its resolve and command go back to it.
(define (lsp/exec-command pane server cmd-obj)
  (lsp-request! pane "workspace/executeCommand"
    (hash "command" (json-ref cmd-obj "command")
          "arguments" (json-ref-or cmd-obj (list) "arguments"))
    (lambda (err res) (when err (lsp/report-error! "code action" err)))
    #:to server
    #:allow-stale #t))

(define (lsp/run-action pane server action gen #:resolved? [resolved? #f])
  (let ((edit (json-ref-or action #f "edit"))
        (command (json-ref-or action #f "command")))
    (cond
      ((or edit command)
       (when edit (apply-workspace-edit! pane edit #:expect-generation gen))
       (when command (lsp/exec-command pane server (if (string? command) action command))))
      ((not resolved?)
       (lsp-request! pane "codeAction/resolve" action
         (lambda (err resolved)
           (cond
             (err (lsp/report-error! "code action" err))
             ((void? resolved) (log! 'info lsp/no-edit-message))
             (else (lsp/run-action pane server resolved gen #:resolved? #t))))
         #:to server
         #:allow-stale #t
         #:unavailable 'empty))
      (else (log! 'info lsp/no-edit-message)))))

;; See docs/features.md, "Code actions".
(define (lsp/code-action-params pane servers diags)
  (let ((base (lsp-primary-range-params pane)))
    (map (lambda (server)
           (cons server
                 (hash-insert base "context"
                   (hash "diagnostics"
                         (map (lambda (d) (hash-ref d 'raw))
                              (filter (lambda (d) (equal? (hash-ref d 'server) server)) diags))
                         "triggerKind" 1))))
         servers)))

;; `(server . action)` for every enabled action, in server order.
(define (lsp/offered-actions results)
  (call! "stdlib/concat"
    (map (lambda (r)
           (map (lambda (a) (cons (hash-ref r 'server) a))
                (filter (lambda (a) (not (json-ref-or a #f "disabled")))
                        (json-list (hash-ref r 'result)))))
         (lsp/answered results))))

;; An action's menu row: its title, followed by its server's name when more
;; than one server offered actions.
(define (lsp/action-row offered)
  (let ((several (call! "stdlib/find"
                        (lambda (entry) (not (equal? (car entry) (car (car offered)))))
                        offered)))
    (lambda (entry)
      (let ((title (json-ref (cdr entry) "title")))
        (if several
            (string-append title " (" (lsp-server-name (car entry)) ")")
            title)))))

(define-command! "lsp/code-actions" "Show available code actions for the cursor or selection."
  (lambda (pane)
    (lsp/with-servers pane "code actions"
      (lambda (servers)
        (let* ((gen (buffer-generation pane))
               (diags (diagnostics-for-buffer pane #:range (lsp/primary-selection-range pane))))
          (lsp-request-all! pane "textDocument/codeAction"
            (lsp/code-action-params pane servers diags)
            (lsp/on-answers "code action"
              (lambda (results)
                (let ((offered (lsp/offered-actions results)))
                  (if (null? offered)
                      (log! 'info "No code actions")
                      (show-menu! pane (lsp/payload-rows (lsp/action-row offered) offered)
                        (lambda (entry)
                          (when entry
                            (lsp/run-action pane (car entry) (cdr entry) gen))))))))
            #:require-focus #t)))
      #:feature 'code-action)))
