;;; core:lsp/actions.scm — textDocument/codeAction.
;;;
;;; context.diagnostics must echo back the *raw* wire Diagnostic objects
;;; currently shown in the request range — servers (rust-analyzer
;;; confirmed) gate diagnostic-derived quickfixes ("remove unused import",
;;; etc.) on this, treating an empty array as "the client isn't showing
;;; any diagnostics here" and withholding them. `diagnostics-for-buffer`'s
;;; `"raw"` field carries these through unmodified — Steel
;;; never reconstructs or re-encodes a wire position itself.

(require "lib.scm")

;;; The primary selection's char range, normalized to (start end) with
;;; end exclusive, for `diagnostics-for-buffer`'s `#:range` filter.
(define (lsp/primary-selection-range)
  (let* ((primary (car (filter caddr (current-selections))))
         (a (car primary))
         (h (cadr primary)))
    (list (min a h) (+ (max a h) 1))))

;;; A CodeAction is disabled if it carries a truthy "disabled" field
;;; (LSP 3.16: `{"reason": string}`) — v1 doesn't pre-filter by `kind`, but
;;; a disabled action must never appear in the menu.
(define (lsp/action-disabled? action)
  (and (hash-contains? action "disabled")
       (not (equal? (hash-ref action "disabled") #f))))

(define (lsp/action-title action)
  (hash-ref action "title"))

;;; `cmd-obj`: a Command `{title, command, arguments?}` (either the bare
;;; top-level shape or a CodeAction's nested "command" field).
(define (lsp/exec-command cmd-obj)
  (lsp-request #f "workspace/executeCommand"
    (hash "command" (hash-ref cmd-obj "command")
          "arguments" (if (hash-contains? cmd-obj "arguments")
                           (hash-ref cmd-obj "arguments")
                           (list)))
    (lambda (err res) (when err (lsp/report-error "code action" err)))))

;;; Applies `edit` first, then runs `command`, per spec order. A bare
;;; `Command` (legacy shape: "command" is a *string* at the top level, no
;;; "edit" key at all) only ever reaches the command branch.
(define (lsp/run-action action)
  (when (hash-contains? action "edit")
    (apply-workspace-edit! (hash-ref action "edit")))
  (when (hash-contains? action "command")
    (let ((cmd (hash-ref action "command")))
      (lsp/exec-command (if (string? cmd) action cmd)))))

(define-command! "lsp-code-actions" "Show available code actions for the cursor or selection."
  (lambda ()
    (let ((bid (current-buffer)))
      (lsp/guard-capability "codeActionProvider"
        (lambda ()
          (let* ((diags (diagnostics-for-buffer bid #:range (lsp/primary-selection-range)))
                 (context (hash "diagnostics" (map (lambda (d) (hash-ref d "raw")) diags)
                                "triggerKind" 1)))
            (lsp-request #f "textDocument/codeAction"
              (hash-insert (lsp-range-params bid) "context" context)
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
