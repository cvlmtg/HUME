;;; core:lsp/inlay.scm — textDocument/inlayHint. See README.md "How it
;;; works" → "Inlay hints". Off by default — `:set global
;;; lsp.inlay-hints=true` opts in.

(require "lib.scm")

;;; `hint`'s label, resolved to plain text — either a bare string or an
;;; `InlayHintLabelPart[]` (each part's "value" concatenated in order).
(define (lsp/inlay-hint-text hint)
  (let ((label (hash-ref hint "label")))
    (if (string? label)
        label
        (string-join (map (lambda (part) (hash-ref part "value")) label) ""))))

;;; `InlayHint {position, label, paddingLeft?, paddingRight?}` -> the
;;; `(offset text 'before)` shape `set-inlay-hints!` expects, or `#f` if
;;; `bid` has no attached server to convert the wire position with (a race
;;; between the request firing and the buffer detaching before the response
;;; arrives).
(define (lsp/hint->store-entry bid hint)
  (let* ((text (lsp/inlay-hint-text hint))
         (pad-left (and (hash-contains? hint "paddingLeft") (equal? (hash-ref hint "paddingLeft") #t)))
         (pad-right (and (hash-contains? hint "paddingRight") (equal? (hash-ref hint "paddingRight") #t)))
         (text (if pad-left (string-append " " text) text))
         (text (if pad-right (string-append text " ") text))
         (offset (lsp-position->offset bid (hash-ref hint "position"))))
    (and offset (list offset text 'before))))

;;; `(first end)` visible lines, 0-based end-exclusive -> `InlayHintParams`,
;;; or `#f` if `bid` can't be resolved (hidden/detached by the time a
;;; debounced refresh fires).
(define (lsp/inlay-hint-params bid first end)
  (let ((pp (lsp-position-params bid)))
    (and pp
         (hash "textDocument" (hash-ref pp "textDocument")
               "range" (hash "start" (hash "line" first "character" 0)
                              "end" (hash "line" end "character" 0))))))

;;; Debounced (200ms) per buffer, re-run from on-viewport-change,
;;; on-diagnostics-changed, and on-text-changed. `debounce-by`, not
;;; `debounce` — see README.
(define lsp/refresh-hints
  (debounce-by 200
    (lambda (bid)
      (let ((range (viewport-range bid))
            (server (lsp-server-for-buffer bid)))
        (when (and range server (get-option "lsp.inlay-hints")
                   (lsp/supports-for-buffer? bid "inlayHintProvider"))
          (let ((params (lsp/inlay-hint-params bid (car range) (cdr range))))
            (when params
              (lsp-request server "textDocument/inlayHint" params
                (lambda (err res)
                  ;; A legitimate empty/null response must still clear any
                  ;; hints from a prior larger one — only an error leaves
                  ;; the existing display untouched.
                  (unless err
                    (set-inlay-hints! "lsp-inlay-hints" bid
                      (if (or (void? res) (null? res))
                          '()
                          (filter (lambda (e) e)
                                  (map (lambda (h) (lsp/hint->store-entry bid h)) res))))))))))))))

(register-hook! 'on-viewport-change
  (lambda (bid first end) (lsp/refresh-hints bid)))

(register-hook! 'on-diagnostics-changed
  (lambda (bid) (lsp/refresh-hints bid)))

;;; Covers undo/redo (and any other edit that doesn't scroll the viewport or
;;; provoke a diagnostics republish) — a hint dropped because its anchor
;;; character was deleted must come back once that edit is undone, not stay
;;; gone until an unrelated scroll or diagnostics event happens to refresh it.
(register-hook! 'on-text-changed
  (lambda (bid) (lsp/refresh-hints bid)))

;;; `refresh-hints` silently skips once `bid` has no attached server, so
;;; stale hints would otherwise sit rendered forever — clear explicitly.
(register-hook! 'on-lsp-detach
  (lambda (bid server-name) (set-inlay-hints! "lsp-inlay-hints" bid '())))

;;; The render bridge is deliberately ungated on `lsp.inlay-hints` — see
;;; README. `value` (the raw `:set`/`set-option!` string) is ignored in
;;; favor of `get-option`'s already-coerced bool.
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.inlay-hints")
      (if (get-option "lsp.inlay-hints")
          (for-each lsp/refresh-hints (buffers))
          (for-each (lambda (bid) (set-inlay-hints! "lsp-inlay-hints" bid '()))
                    (buffers))))))
