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

(define (lsp/resolve-pane pane)
  (let ((panes (buffer-panes pane)))
    (if (null? panes) #f (car panes))))

;; ── Viewport ────────────────────────────────────────────────────────────────

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

(define (lsp/show-locations! locs)
  (show-drawer-list! (focused-pane) (map lsp/location-display (lsp-locations->display-parts locs))
    (lambda (idx) (when idx (goto-location! (focused-pane) (list-ref locs idx))))))
