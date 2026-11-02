;;; core:lsp/inlay.scm — textDocument/inlayHint. See docs/decorations.md.

(require "helpers.scm")

(define lsp/inlay-default
  (call! "stdlib/config-boolean" "core:lsp" (plugin-config) 'inlay-hints #f))

(define lsp/inlay-source "lsp-inlay-hints")

;;; Buffers toggled away from `lsp/inlay-default`, keyed by `(buffer-key pane)`.
(define lsp/*inlay-overrides* (hash))

(define lsp/inlay-key (lsp-supersede-key))

(define (lsp/inlay-on? pane)
  (let ((key (buffer-key pane)))
    (if (hash-contains? lsp/*inlay-overrides* key)
        (hash-ref lsp/*inlay-overrides* key)
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

(define (lsp/inlay-hint-params pane range)
  (let ((params (lsp-document-params pane)))
    (and params
         (hash-insert params "range"
           (hash "start" (hash "line" (hash-ref range 'start-line) "character" 0)
                 "end" (hash "line" (hash-ref range 'end-line) "character" 0))))))

;; A pane showing `pane`'s buffer, or #f when none does or the buffer has closed.
(define (lsp/resolve-pane pane)
  (and (buffer-live? pane)
       (let ((panes (buffer-panes pane)))
         (if (null? panes) #f (car panes)))))

;;; `pane` is any pane value of the buffer; the pane is resolved when the debounce fires.
;; See docs/decorations.md, "Inlay hints".
(define lsp/request-hints
  (debounce-by 200
    (lambda (pane)
      (let ((pane (lsp/resolve-pane pane)))
        (when (and pane (lsp/inlay-on? pane))
          (let ((params (lsp/inlay-hint-params pane (viewport-range pane))))
            (when params
              (lsp-request-all! pane "textDocument/inlayHint" params
                (lambda (err results)
                  (lsp/report-answer-errors! "inlay hints" results)
                  ;; See docs/decorations.md, "Inlay hints", on the response.
                  (unless (or (not (lsp/inlay-on? pane)) (lsp/none-answered? results))
                    (set-inlay-hints! pane lsp/inlay-source
                      (filter (lambda (e) e)
                              (map (lambda (h) (lsp/hint->store-entry pane h))
                                   (lsp/answer-items results))))))
                #:unavailable 'empty
                #:supersede lsp/inlay-key))))))
    #:key (lambda (p . _) (buffer-key p))))

(define (lsp/refresh-hints pane)
  (when (and (lsp/inlay-on? pane) (lsp/resolve-pane pane))
    (lsp/request-hints pane)))

(register-hook! 'on-viewport-change
  (lambda (pane range) (lsp/refresh-hints pane)))

(register-hook! 'on-lsp-attach
  (lambda (pane server) (lsp/refresh-hints pane)))

(register-hook! 'on-inlay-hints-refresh
  (lambda (pane server) (lsp/refresh-hints pane)))

(register-hook! 'on-text-changed
  (lambda (pane changes) (lsp/refresh-hints pane)))

(register-hook! 'on-lsp-detach
  (lambda (pane server)
    (if (lsp/resolve-pane pane)
        (lsp/refresh-hints pane)
        (set-inlay-hints! pane lsp/inlay-source '()))))

(register-hook! 'on-buffer-close
  (lambda (pane)
    (set! lsp/*inlay-overrides* (hash-remove lsp/*inlay-overrides* (buffer-key pane)))))

(define-typed-command! "toggle-inlay-hints"
  "Toggle LSP inlay hints for the current buffer."
  (lambda (pane)
    (let ((on? (not (lsp/inlay-on? pane))))
      (set! lsp/*inlay-overrides* (hash-insert lsp/*inlay-overrides* (buffer-key pane) on?))
      (if on?
          (lsp/request-hints pane)
          (set-inlay-hints! pane lsp/inlay-source '()))
      (log! 'info (if on? "inlay hints on" "inlay hints off")))))
