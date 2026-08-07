;;; core:lsp/inlay.scm — textDocument/inlayHint.
;;;
;;; Off by default — `:set global lsp.inlay-hints=true` opts in.

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
;;; arrives). Padding becomes literal leading/trailing spaces.
(define (lsp/hint->store-entry bid hint)
  (let* ((text (lsp/inlay-hint-text hint))
         (pad-left (and (hash-contains? hint "paddingLeft") (equal? (hash-ref hint "paddingLeft") #t)))
         (pad-right (and (hash-contains? hint "paddingRight") (equal? (hash-ref hint "paddingRight") #t)))
         (text (if pad-left (string-append " " text) text))
         (text (if pad-right (string-append text " ") text))
         (offset (lsp-position->offset bid (hash-ref hint "position"))))
    (and offset (list offset text 'before))))

;;; `(first last)` visible lines -> `InlayHintParams`, or `#f` if `bid` can't
;;; be resolved (hidden/detached by the time a debounced refresh fires).
(define (lsp/inlay-hint-params bid first last)
  (let ((pp (lsp-position-params bid)))
    (and pp
         (hash "textDocument" (hash-ref pp "textDocument")
               "range" (hash "start" (hash "line" first "character" 0)
                              "end" (hash "line" (+ last 1) "character" 0))))))

;;; Debounced (200ms) per buffer, re-run from both on-viewport-change and
;;; on-diagnostics-changed. `debounce-by`, not `debounce`: keying per `bid`
;;; keeps a diagnostics batch touching two buffers from having the second
;;; buffer's call cancel the first's. Re-reads the live viewport rather than
;;; trusting an argument, so both hooks can share one signature.
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
  (lambda (bid first last) (lsp/refresh-hints bid)))

(register-hook! 'on-diagnostics-changed
  (lambda (bid) (lsp/refresh-hints bid)))

;;; `refresh-hints` silently skips once `bid` has no attached server, so
;;; stale hints would otherwise sit rendered forever — clear explicitly.
(register-hook! 'on-lsp-detach
  (lambda (bid server-name) (set-inlay-hints! "lsp-inlay-hints" bid '())))

;;; The render bridge (`update_inlay_hint_providers`) is not gated on
;;; `lsp.inlay-hints` — the store is per-source, so an unrelated plugin's
;;; hints must not vanish just because this setting toggles. This plugin
;;; owns clearing *its own* source when the setting turns off, and
;;; re-requesting hints for every visible buffer when it turns back on.
;;; `value` (the raw `:set`/`set-option!` string) is ignored in favor of
;;; `get-option`'s already-coerced bool — `value` could be "on"/"yes"/"1",
;;; any of `parse-bool`'s accepted spellings, not just the literal "true".
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "lsp.inlay-hints")
      (if (get-option "lsp.inlay-hints")
          (for-each lsp/refresh-hints (buffers))
          (for-each (lambda (bid) (set-inlay-hints! "lsp-inlay-hints" bid '()))
                    (buffers))))))
