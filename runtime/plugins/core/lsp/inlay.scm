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
    (and offset (hash 'offset offset 'text text 'side 'before))))

(define (lsp/inlay-hint-params pane first end)
  (let ((pp (lsp-position-params pane)))
    (and pp
         (hash "textDocument" (hash-ref pp "textDocument")
               "range" (hash "start" (hash "line" first "character" 0)
                              "end" (hash "line" end "character" 0))))))

;;; `pane` must already be a real, live pane — see docs/decorations.md's "Inlay hints".
;; Every server's hints merge into one set; a buffer left with no server
;; giving hints is cleared.
(define lsp/refresh-hints
  (debounce-by 200
    (lambda (pane)
      (let ((range (viewport-range pane)))
        (when (get-option "lsp.inlay-hints")
          (let ((params (lsp/inlay-hint-params pane (hash-ref range 'start) (hash-ref range 'end))))
            (when params
              (lsp-request-all! pane "textDocument/inlayHint" params
                (lambda (err results)
                  (when err (lsp/report-error! "inlay hints" err))
                  (lsp/report-answer-errors! "inlay hints" results)
                  ;; When every server failed the hints already shown stay (docs/decorations.md).
                  (unless (or err (and (pair? results) (lsp/none-answered? results)))
                    (set-inlay-hints! "lsp-inlay-hints" pane
                      (filter (lambda (e) e)
                              (map (lambda (h) (lsp/hint->store-entry pane h))
                                   (apply append (map json-list (lsp/answers results))))))))
                #:unavailable 'empty))))))
    #:key (lambda (p . _) (buffer-key p))))

;;; `pane` need not itself be live — see docs/decorations.md's "Inlay hints".
(define (lsp/refresh-hints-for-buffer pane)
  (let ((resolved (lsp/resolve-pane pane)))
    (when resolved (lsp/refresh-hints resolved))))

(register-hook! 'on-viewport-change
  (lambda (pane first end) (lsp/refresh-hints pane)))

(register-hook! 'on-diagnostics-changed lsp/refresh-hints-for-buffer)

(register-hook! 'on-text-changed lsp/refresh-hints-for-buffer)

(register-hook! 'on-lsp-detach
  (lambda (pane server) (lsp/refresh-hints-for-buffer pane)))

(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.inlay-hints")
      (if value
          (for-each lsp/refresh-hints-for-buffer (buffers))
          (for-each (lambda (pane) (set-inlay-hints! "lsp-inlay-hints" pane '()))
                    (buffers))))))
