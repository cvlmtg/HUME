;;; core:lsp/inlay.scm — textDocument/inlayHint. See docs/decorations.md.
;;; Off by default — `:set global lsp.inlay-hints=true` opts in.

(require "lib.scm")

;;; `hint`'s label — a bare string or `InlayHintLabelPart[]`.
(define (lsp/inlay-hint-text hint)
  (let ((label (hash-ref hint "label")))
    (if (string? label)
        label
        (string-join (map (lambda (part) (hash-ref part "value")) label) ""))))

;;; `#f` if `bid` has no attached server to convert the wire position with
;;; (a race between the request firing and the response arriving).
(define (lsp/hint->store-entry bid hint)
  (let* ((text (lsp/inlay-hint-text hint))
         (pad-left (and (hash-contains? hint "paddingLeft") (equal? (hash-ref hint "paddingLeft") #t)))
         (pad-right (and (hash-contains? hint "paddingRight") (equal? (hash-ref hint "paddingRight") #t)))
         (text (if pad-left (string-append " " text) text))
         (text (if pad-right (string-append text " ") text))
         (offset (lsp-position->offset bid (hash-ref hint "position"))))
    (and offset (list offset text 'before))))

;;; `#f` if `bid` can't be resolved (hidden/detached by the time a
;;; debounced refresh fires).
(define (lsp/inlay-hint-params bid first end)
  (let ((pp (lsp-position-params bid)))
    (and pp
         (hash "textDocument" (hash-ref pp "textDocument")
               "range" (hash "start" (hash "line" first "character" 0)
                              "end" (hash "line" end "character" 0))))))

;;; `debounce-by`, not `debounce` — see docs/decorations.md.
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
                  ;; Empty/null must still clear prior hints — only an
                  ;; error leaves the existing display untouched.
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

;;; Covers undo/redo — see docs/decorations.md.
(register-hook! 'on-text-changed
  (lambda (bid) (lsp/refresh-hints bid)))

(register-hook! 'on-lsp-detach
  (lambda (bid server-name) (set-inlay-hints! "lsp-inlay-hints" bid '())))

;;; Deliberately ungated on `lsp.inlay-hints` — see docs/decorations.md.
;;; `value` (the raw `:set`/`set-option!` string) is ignored in favor of
;;; `get-option`'s already-coerced bool.
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.inlay-hints")
      (if (get-option "lsp.inlay-hints")
          (for-each lsp/refresh-hints (buffers))
          (for-each (lambda (bid) (set-inlay-hints! "lsp-inlay-hints" bid '()))
                    (buffers))))))
