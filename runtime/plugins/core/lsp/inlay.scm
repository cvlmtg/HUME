;;; core:lsp/inlay.scm — textDocument/inlayHint. See docs/decorations.md.

(require "lib.scm")

(define (lsp/inlay-hint-text hint)
  (let ((label (json-ref hint "label")))
    (if (string? label)
        label
        (string-join (map (lambda (part) (json-ref part "value")) (json-list label)) ""))))

(define (lsp/hint->store-entry pane hint)
  (let* ((text (lsp/inlay-hint-text hint))
         (pad-left (equal? (json-ref-or hint #f "paddingLeft") #t))
         (pad-right (equal? (json-ref-or hint #f "paddingRight") #t))
         (text (if pad-left (string-append " " text) text))
         (text (if pad-right (string-append text " ") text))
         (offset (lsp-position->offset pane (json-ref hint "position"))))
    (and offset (list offset text 'before))))

(define (lsp/inlay-hint-params pane first end)
  (let ((pp (lsp-position-params pane)))
    (and pp
         (hash "textDocument" (hash-ref pp "textDocument")
               "range" (hash "start" (hash "line" first "character" 0)
                              "end" (hash "line" end "character" 0))))))

;;; `pane` must already be a real, live pane still showing its buffer —
;;; `on-viewport-change` hands one directly; every other caller below
;;; resolves one first via `lsp/resolve-pane`, since `on-diagnostics-
;;; changed`/`on-text-changed`/`(buffers)` carry no pane of their own.
(define lsp/refresh-hints
  (debounce-by 200
    (lambda (pane)
      (let ((range (viewport-range pane)))
        (when (and (get-option "lsp.inlay-hints")
                   (lsp/supports? pane "inlayHintProvider"))
          (let ((params (lsp/inlay-hint-params pane (car range) (cdr range))))
            (when params
              (lsp-request pane "textDocument/inlayHint" params
                (lambda (err res)
                  (unless err
                    (set-inlay-hints! "lsp-inlay-hints" pane
                      (if (void? res)
                          '()
                          (filter (lambda (e) e)
                                  (map (lambda (h) (lsp/hint->store-entry pane h)) (json-list res)))))))))))))
    #:key (lambda (p . _) (buffer-key p))))

;;; `pane` need not itself be live — resolves it via `lsp/resolve-pane`
;;; first, a no-op if nothing shows its buffer anywhere. The shared tail
;;; every hook below reduces to, except `on-viewport-change`, which already
;;; hands a live pane directly (see `lsp/refresh-hints`'s own doc).
(define (lsp/refresh-hints-for-buffer pane)
  (let ((resolved (lsp/resolve-pane pane)))
    (when resolved (lsp/refresh-hints resolved))))

(register-hook! 'on-viewport-change
  (lambda (pane first end) (lsp/refresh-hints pane)))

(register-hook! 'on-diagnostics-changed lsp/refresh-hints-for-buffer)

(register-hook! 'on-text-changed lsp/refresh-hints-for-buffer)

(register-hook! 'on-lsp-detach
  (lambda (pane server-name) (set-inlay-hints! "lsp-inlay-hints" pane '())))

(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.inlay-hints")
      (if (get-option "lsp.inlay-hints")
          (for-each lsp/refresh-hints-for-buffer (buffers))
          (for-each (lambda (pane) (set-inlay-hints! "lsp-inlay-hints" pane '()))
                    (buffers))))))
