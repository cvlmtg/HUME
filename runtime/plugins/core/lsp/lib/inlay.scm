;;; core:lsp/inlay.scm — textDocument/inlayHint. See docs/decorations.md.

(require "helpers.scm")

(define lsp/inlay-default
  (call! "stdlib/config-boolean" "core:lsp" (plugin-config) 'inlay-hints #f))

(define lsp/inlay-source "lsp-inlay-hints")

;;; Buffers toggled away from `lsp/inlay-default`, keyed by `(buffer-key pane)`.
(define lsp/*inlay-overrides* (hash))

;;; Each buffer's round of hint requests is one batch under this key.
(define lsp/inlay-batch (call! "stdlib/lsp-batch-key"))

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
         ;; See docs/decorations.md, "Inlay hints", on padding.
         (pad-left (and (equal? (json-ref-or hint #f "paddingLeft") #t)
                        (not (starts-with? text ":"))))
         (pad-right (equal? (json-ref-or hint #f "paddingRight") #t))
         (text (if pad-left (string-append " " text) text))
         (text (if pad-right (string-append text " ") text))
         (offset (lsp-position->char-offset pane (json-ref hint "position"))))
    (and offset (hash 'char-offset offset 'text text 'side 'before))))

(define (lsp/hint-in-range? hint range)
  (let ((line (json-ref hint "position" "line")))
    (and (<= (hash-ref range 'start-line) line) (< line (hash-ref range 'end-line)))))

(define (lsp/inlay-hint-params pane range)
  (hash-insert (lsp-document-params pane) "range"
    (hash "start" (hash "line" (hash-ref range 'start-line) "character" 0)
          "end" (hash "line" (hash-ref range 'end-line) "character" 0))))

;; A pane showing `pane`'s buffer, or #f when none does or the buffer has closed.
(define (lsp/resolve-pane pane)
  (and (buffer-live? pane)
       (let ((panes (buffer-panes pane)))
         (if (null? panes) #f (car panes)))))

;; `answers` pairs each range of a round with its `lsp-request-all!` results.
;; See docs/decorations.md, "Inlay hints", on the response.
(define (lsp/set-round-hints! pane answers)
  (unless (or (not (lsp/inlay-on? pane))
              (call! "stdlib/find" (lambda (a) (lsp/none-answered? (cdr a))) answers))
    (set-inlay-hints! pane lsp/inlay-source
      (filter (lambda (e) e)
              (call! "stdlib/concat"
                (map (lambda (a)
                       (map (lambda (h) (lsp/hint->store-entry pane h))
                            (filter (lambda (h) (lsp/hint-in-range? h (car a)))
                                    (lsp/answer-items (cdr a)))))
                     answers))))))

(define (lsp/request-round! pane ranges)
  (call! "stdlib/lsp-batch" pane
    (map (lambda (range)
           (lambda (key done)
             (lsp-request-all! pane "textDocument/inlayHint"
               (lsp/inlay-hint-params pane range)
               done
               #:unavailable 'empty
               #:supersede key)))
         ranges)
    (lambda (answers)
      (let ((results (map cadr answers)))
        (for-each (lambda (r) (lsp/report-answer-errors! "inlay hints" r)) results)
        (lsp/set-round-hints! pane (map cons ranges results))))
    (hash 'supersede lsp/inlay-batch)))

;;; `pane` is any pane value of the buffer; the pane is resolved when the debounce fires.
;; See docs/decorations.md, "Inlay hints".
(define lsp/request-hints
  (debounce-by 200
    (lambda (pane)
      (let ((pane (lsp/resolve-pane pane)))
        (when (and pane (lsp/inlay-on? pane))
          (lsp/request-round! pane (hash-ref (buffer-viewport-ranges pane) 'ranges)))))
    #:key (lambda (p . _) (buffer-key p))))

(define (lsp/refresh-hints pane)
  (when (and (lsp/inlay-on? pane) (lsp/resolve-pane pane))
    (lsp/request-hints pane)))

(register-hook! 'on-viewport-change
  (lambda (pane range headers) (lsp/refresh-hints pane)))

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
    (set! lsp/*inlay-overrides* (hash-remove lsp/*inlay-overrides* (buffer-key pane)))
    (when (buffer-live? pane)
      (set-inlay-hints! pane lsp/inlay-source '()))))

(define-typed-command! "toggle-inlay-hints"
  "Toggle LSP inlay hints for the current buffer."
  (lambda (pane)
    (let ((on? (not (lsp/inlay-on? pane))))
      (set! lsp/*inlay-overrides* (hash-insert lsp/*inlay-overrides* (buffer-key pane) on?))
      (if on?
          (lsp/request-hints pane)
          (set-inlay-hints! pane lsp/inlay-source '()))
      (log! 'info (if on? "inlay hints on" "inlay hints off")))))
