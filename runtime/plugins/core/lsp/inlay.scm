;;; core:lsp/inlay.scm — textDocument/inlayHint. See docs/decorations.md.

(require "lib.scm")

(define (lsp/inlay-hint-text hint)
  (let ((label (json-ref hint "label")))
    (if (string? label)
        label
        (string-join (map (lambda (part) (json-ref part "value")) (json-list label)) ""))))

(define (lsp/hint->store-entry bid hint)
  (let* ((text (lsp/inlay-hint-text hint))
         (pad-left (equal? (json-ref-or hint #f "paddingLeft") #t))
         (pad-right (equal? (json-ref-or hint #f "paddingRight") #t))
         (text (if pad-left (string-append " " text) text))
         (text (if pad-right (string-append text " ") text))
         (offset (lsp-position->offset bid (json-ref hint "position"))))
    (and offset (list offset text 'before))))

(define (lsp/inlay-hint-params bid first end)
  (let ((pp (lsp-position-params bid)))
    (and pp
         (hash "textDocument" (hash-ref pp "textDocument")
               "range" (hash "start" (hash "line" first "character" 0)
                              "end" (hash "line" end "character" 0))))))

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
                  (unless err
                    (set-inlay-hints! "lsp-inlay-hints" bid
                      (if (void? res)
                          '()
                          (filter (lambda (e) e)
                                  (map (lambda (h) (lsp/hint->store-entry bid h)) (json-list res)))))))))))))))

(register-hook! 'on-viewport-change
  (lambda (bid first end) (lsp/refresh-hints bid)))

(register-hook! 'on-diagnostics-changed
  (lambda (bid) (lsp/refresh-hints bid)))

(register-hook! 'on-text-changed
  (lambda (bid) (lsp/refresh-hints bid)))

(register-hook! 'on-lsp-detach
  (lambda (bid server-name) (set-inlay-hints! "lsp-inlay-hints" bid '())))

(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.inlay-hints")
      (if (get-option "lsp.inlay-hints")
          (for-each lsp/refresh-hints (buffers))
          (for-each (lambda (bid) (set-inlay-hints! "lsp-inlay-hints" bid '()))
                    (buffers))))))
