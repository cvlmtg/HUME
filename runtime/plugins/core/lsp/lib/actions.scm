;;; core:lsp/actions.scm — textDocument/codeAction. See docs/features.md.

(require "helpers.scm")

(define (lsp/primary-selection-range pane)
  (let ((primary (call! "stdlib/primary-selection" (buffer-selections pane))))
    (and primary
         (hash 'start (call! "stdlib/selection-start" primary)
              'end (call! "stdlib/selection-end" primary)))))

(define (lsp/action-disabled? action)
  (not (equal? (json-ref-or action #f "disabled") #f)))

(define (lsp/action-title action)
  (json-ref action "title"))

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
             ((and err (equal? (hash-ref err 'kind) 'unavailable))
              (log! 'info "Code action has no edit or command"))
             (err (lsp/report-error! "code action" err))
             ((void? resolved) (log! 'info "Code action has no edit or command"))
             (else (lsp/run-action pane server resolved gen #:resolved? #t))))
         #:to server
         #:allow-stale #t))
      (else (log! 'info "Code action has no edit or command")))))

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
  (apply append
    (map (lambda (r)
           (map (lambda (a) (cons (hash-ref r 'server) a))
                (filter (lambda (a) (not (lsp/action-disabled? a)))
                        (json-list (hash-ref r 'result)))))
         (lsp/answered results))))

;; An action's menu row: its title, followed by its server's name when more
;; than one server offered actions.
(define (lsp/action-row offered)
  (let ((several (let loop ((rest offered))
                   (cond ((null? rest) #f)
                         ((equal? (car (car rest)) (car (car offered))) (loop (cdr rest)))
                         (else #t)))))
    (lambda (entry)
      (if several
          (string-append (lsp/action-title (cdr entry)) " (" (lsp-server-name (car entry)) ")")
          (lsp/action-title (cdr entry))))))

(define-command! "lsp-code-actions" "Show available code actions for the cursor or selection."
  (lambda (pane)
    (lsp/with-servers (lsp-servers pane #:feature 'code-action) "code actions"
      (lambda (servers)
        (let* ((gen (buffer-generation pane))
               (diags (diagnostics-for-buffer pane #:range (lsp/primary-selection-range pane))))
          (lsp-request-all! pane "textDocument/codeAction"
            (lsp/code-action-params pane servers diags)
            (lambda (err results)
              (if err
                  (lsp/report-error! "code action" err)
                  (let ((offered (lsp/offered-actions results)))
                    (lsp/report-answer-errors! "code action" results)
                    (if (null? offered)
                        (log! 'info "No code actions")
                        (show-menu! pane (map (lsp/action-row offered) offered)
                          (lambda (idx)
                            (when idx
                              (let ((entry (list-ref offered idx)))
                                (lsp/run-action pane (car entry) (cdr entry) gen)))))))))
            #:require-focus #t))))))
