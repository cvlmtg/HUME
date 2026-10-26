;;; core:lsp/inlay.scm — textDocument/inlayHint. See docs/decorations.md.

(require "helpers.scm")

(define lsp/inlay-default
  (call! "stdlib/config-boolean" "core:lsp" (plugin-config) "inlay-hints" #f))

;;; Buffers toggled away from `lsp/inlay-default`, keyed by `(buffer-key pane)`.
(define lsp/*inlay-overrides* (box (hash)))

;;; Each buffer's supersede key, keyed by `(buffer-key pane)`.
(define lsp/*inlay-keys* (box (hash)))

(define (lsp/inlay-key pane)
  (let* ((keys (unbox lsp/*inlay-keys*))
         (bkey (buffer-key pane)))
    (or (hash-try-get keys bkey)
        (let ((key (lsp-supersede-key)))
          (set-box! lsp/*inlay-keys* (hash-insert keys bkey key))
          key))))

(define (lsp/inlay-on? pane)
  (let ((overrides (unbox lsp/*inlay-overrides*))
        (key (buffer-key pane)))
    (if (hash-contains? overrides key)
        (hash-ref overrides key)
        lsp/inlay-default)))

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
         (offset (lsp-position->char-offset pane (json-ref hint "position"))))
    (and offset (hash 'char-offset offset 'text text 'side 'before))))

(define (lsp/inlay-hint-params pane first end)
  (let ((pp (lsp-position-params pane)))
    (and pp
         (hash "textDocument" (hash-ref pp "textDocument")
               "range" (hash "start" (hash "line" first "character" 0)
                              "end" (hash "line" end "character" 0))))))

;;; `pane` must already be a real, live pane — see docs/decorations.md's "Inlay hints".
;; See docs/decorations.md, "Inlay hints".
(define lsp/refresh-hints
  (debounce-by 200
    (lambda (pane)
      (let ((range (viewport-range pane)))
        (when (lsp/inlay-on? pane)
          (let ((params (lsp/inlay-hint-params pane (hash-ref range 'start-line) (hash-ref range 'end-line))))
            (when params
              (lsp-request-all! pane "textDocument/inlayHint" params
                (lambda (err results)
                  (when err (lsp/report-error! "inlay hints" err))
                  (lsp/report-answer-errors! "inlay hints" results)
                  ;; See docs/decorations.md, "Inlay hints", on the response.
                  (unless (or err (not (lsp/inlay-on? pane))
                              (and (pair? results) (lsp/none-answered? results)))
                    (set-inlay-hints! "lsp-inlay-hints" pane
                      (filter (lambda (e) e)
                              (map (lambda (h) (lsp/hint->store-entry pane h))
                                   (lsp/answer-items results))))))
                #:unavailable 'empty
                #:supersede (lsp/inlay-key pane)))))))
    #:key (lambda (p . _) (buffer-key p))))

;;; `pane` need not itself be live — see docs/decorations.md's "Inlay hints".
(define (lsp/refresh-hints-for-buffer pane)
  (let ((resolved (lsp/resolve-pane pane)))
    (when resolved (lsp/refresh-hints resolved))))

(register-hook! 'on-viewport-change
  (lambda (pane range) (lsp/refresh-hints pane)))

(register-hook! 'on-diagnostics-changed lsp/refresh-hints-for-buffer)

(register-hook! 'on-text-changed lsp/refresh-hints-for-buffer)

(register-hook! 'on-lsp-detach
  (lambda (pane server)
    (if (lsp/resolve-pane pane)
        (lsp/refresh-hints-for-buffer pane)
        (set-inlay-hints! "lsp-inlay-hints" pane '()))))

(register-hook! 'on-buffer-close
  (lambda (pane)
    (let ((bkey (buffer-key pane)))
      (set-box! lsp/*inlay-overrides*
                (hash-remove (unbox lsp/*inlay-overrides*) bkey))
      (set-box! lsp/*inlay-keys*
                (hash-remove (unbox lsp/*inlay-keys*) bkey)))))

(define-typed-command! "toggle-inlay-hints"
  "Toggle LSP inlay hints for the current buffer."
  (lambda (pane)
    (let ((on? (not (lsp/inlay-on? pane))))
      (set-box! lsp/*inlay-overrides*
                (hash-insert (unbox lsp/*inlay-overrides*) (buffer-key pane) on?))
      (if on?
          (lsp/refresh-hints pane)
          (set-inlay-hints! "lsp-inlay-hints" pane '()))
      (log! 'info (if on? "inlay hints on" "inlay hints off")))))
