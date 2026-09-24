;;; core:lsp/lib.scm — shared helpers used by every feature file. See
;;; docs/architecture.md.

(provide lsp/supports? lsp/guard-capability lsp/report-error
         lsp/visible-lines lsp/show-locations! lsp/resolve-pane
         lsp/setup-trigger-chars! lsp/format-position lsp/cap-field lsp/cap-flag?)

;; ── Capability guard ────────────────────────────────────────────────────────

(define (lsp/caps-has-cap? caps cap-key)
  (and caps
       (let ((v (json-ref-or caps #f cap-key)))
         (not (or (equal? v #f) (void? v))))))

(define (lsp/supports? pane cap-key)
  (lsp/caps-has-cap? (lsp-capabilities pane) cap-key))

(define (lsp/guard-capability pane cap-key thunk)
  (if (lsp/supports? pane cap-key)
      (thunk)
      (log! 'info
            (string-append "not supported by "
                           (let ((name (lsp-server-for-buffer pane)))
                             (if name name "server"))))))

(define (lsp/cap-field caps cap-key field default)
  (let ((cap (and caps (json-ref-or caps #f cap-key))))
    (if (json-object? cap) (json-ref-or cap default field) default)))

(define (lsp/cap-flag? pane cap-key field)
  (equal? (lsp/cap-field (lsp-capabilities pane) cap-key field #f) #t))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

(define (lsp/setup-trigger-chars! cap-key source-name extra-chars on-trigger)
  (define (set-chars! server-name chars)
    (if on-trigger
        (register-trigger-chars! source-name server-name chars)
        (completion-set-trigger-chars! source-name server-name chars)))
  (register-hook! 'on-lsp-attach
    (lambda (pane server-name)
      (let ((caps (lsp-capabilities pane)))
        (when (and caps (json-contains? caps cap-key))
          (let ((tc (lsp/cap-field caps cap-key "triggerCharacters" #f)))
            (set-chars! server-name (append extra-chars (if tc (json-list tc) (list)))))))))
  (register-hook! 'on-lsp-detach
    (lambda (pane server-name)
      (set-chars! server-name '())))
  (when on-trigger
    (register-hook! 'on-trigger-char
      (lambda (pane ch source)
        (when (equal? source source-name)
          (on-trigger pane ch))))))

(define (lsp/report-error what err)
  (log! 'error
        (string-append "lsp " what ": "
                       (if (string? err) err (hash-ref err "message")))))

;; ── Pane resolution ──────────────────────────────────────────────────────────

;;; `pane` resolved to a real pane still showing its buffer — `(car
;;; (buffer-panes pane))`, the focused pane if it shows the buffer, else the
;;; first pane on the active tab, else any other; `#f` if the buffer isn't
;;; shown anywhere. The explicit choice a hook whose own value carries no
;;; pane (`on-diagnostics-changed`, `on-text-changed`, a `(buffers)`
;;; element) makes before calling a pane-needing builtin
;;; (`viewport-range`, `lsp-position-params`, …), in place of the implicit
;;; guess those builtins used to make internally. A caller already holding
;;; a real pane (`on-viewport-change`, `on-trigger-char`, a command's own
;;; leading `pane`) has no reason to call this — re-resolving would risk
;;; silently picking a *different* pane on the same buffer.
(define (lsp/resolve-pane pane)
  (let ((panes (buffer-panes pane)))
    (if (null? panes) #f (car panes))))

;; ── Viewport ────────────────────────────────────────────────────────────────

;;; `pane` must already be a real, live pane still showing its buffer —
;;; `viewport-range` raises otherwise (kind-B fail-fast), unlike the old
;;; pane-only builtin this replaces, which answered `#f` for "not shown
;;; anywhere". Every current caller passes a command's own leading `pane`
;;; (always the focused one) or a `lsp/resolve-pane`-resolved value, so
;;; there is no "not shown" case left to degrade gracefully.
(define (lsp/visible-lines pane)
  (let ((range (viewport-range pane)))
    (- (cdr range) (car range))))

;; ── Location display + drawer ───────────────────────────────────────────────

(define (lsp/format-position line col)
  (string-append (number->string (+ 1 line)) ":" (number->string (+ 1 col))))

(define (lsp/location-display part)
  (let* ((path (path->display (car part)))
         (line (cadr part))
         (grapheme-col-or-wire (caddr part)))
    (string-append path ":"
      (if grapheme-col-or-wire
          (lsp/format-position line grapheme-col-or-wire)
          (number->string (+ 1 line))))))

;;; Each entry in `locs` carries its own tagged producing-server encoding
;;; (its `JsonHandle`, from the response `lsp/goto-response` decoded) —
;;; `lsp-locations->display-parts` and `goto-location!` both read it
;;; straight off the handle, never from a captured pane.
;;;
;;; `(focused-pane)`, not the request's own invocation pane: `lsp/goto-
;;; response`'s callers deliberately skip `#:require-focus` so a slow
;;; goto/references response still completes even if the user looked
;;; elsewhere meanwhile — `show-drawer-list!` needs the *focused* pane, so
;;; this opens (and later jumps) wherever the user actually is once the
;;; response lands, the same "wherever focus is now" behavior this had
;;; before panes existed, rather than raising when the original pane is no
;;; longer focused or has since closed.
(define (lsp/show-locations! locs)
  (show-drawer-list! (focused-pane) (map lsp/location-display (lsp-locations->display-parts locs))
    (lambda (idx) (when idx (goto-location! (focused-pane) (list-ref locs idx))))))
