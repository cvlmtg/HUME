;;; core:lsp/lib.scm — shared helpers used by every feature file. See
;;; docs/architecture.md.

(provide lsp/supports? lsp/supports-for-buffer? lsp/guard-capability lsp/report-error
         lsp/visible-lines lsp/show-locations! lsp/text-edit->tuple
         lsp/setup-trigger-chars! lsp/format-position lsp/cap-field lsp/cap-flag?)

;; ── Capability guard ────────────────────────────────────────────────────────

(define (lsp/caps-has-cap? caps cap-key)
  (and caps
       (hash-contains? caps cap-key)
       (not (equal? (hash-ref caps cap-key) #f))))

(define (lsp/supports? cap-key)
  (lsp/caps-has-cap? (lsp-capabilities #f) cap-key))

;;; #f when `bid` has no attached server at all.
(define (lsp/supports-for-buffer? bid cap-key)
  (let ((server (lsp-server-for-buffer bid)))
    (and server (lsp/caps-has-cap? (lsp-capabilities server) cap-key))))

(define (lsp/guard-capability cap-key thunk)
  (if (lsp/supports? cap-key)
      (thunk)
      (log! 'info
            (string-append "not supported by "
                           (let ((name (lsp-server-for-buffer (current-buffer))))
                             (if name name "server"))))))

(define (lsp/cap-field caps cap-key field default)
  (if (and caps (hash-contains? caps cap-key))
      (let ((cap (hash-ref caps cap-key)))
        (if (and (hash? cap) (hash-contains? cap field))
            (hash-ref cap field)
            default))
      default))

(define (lsp/cap-flag? cap-key field)
  (equal? (lsp/cap-field (lsp-capabilities #f) cap-key field #f) #t))

;; ── Popup dismissal ─────────────────────────────────────────────────────────
;; Shared by every feature using a popup (hover, sighelp, …).
(register-hook! 'on-mode-change (lambda (old-mode new-mode) (close-popup!)))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

(define (lsp/setup-trigger-chars! cap-key source-name extra-chars on-trigger)
  (register-hook! 'on-lsp-attach
    (lambda (bid server-name)
      (let ((caps (lsp-capabilities server-name)))
        (when (and caps (hash-contains? caps cap-key))
          (register-trigger-chars! source-name server-name
            (append extra-chars (lsp/cap-field caps cap-key "triggerCharacters" (list))))))))
  (register-hook! 'on-lsp-detach
    (lambda (bid server-name)
      (register-trigger-chars! source-name server-name '())))
  (register-hook! 'on-trigger-char
    (lambda (bid ch source)
      (when (equal? source source-name)
        (on-trigger bid ch)))))

(define (lsp/report-error what err)
  (log! 'error
        (string-append "lsp " what ": "
                       (if (string? err) err (hash-ref err "message")))))

;;; `TextEdit` hashmap -> the tuple shape `apply-text-edits!` expects.
(define (lsp/text-edit->tuple te)
  (let* ((range (hash-ref te "range"))
         (start (hash-ref range "start"))
         (end (hash-ref range "end")))
    (list (cons (hash-ref start "line") (hash-ref start "character"))
          (cons (hash-ref end "line") (hash-ref end "character"))
          (hash-ref te "newText"))))

;; ── Viewport ────────────────────────────────────────────────────────────────

;;; #f if `bid` isn't shown in any pane.
(define (lsp/visible-lines bid)
  (let ((range (viewport-range bid)))
    (if range (- (cdr range) (car range)) #f)))

;; ── Location display + drawer ───────────────────────────────────────────────

(define (lsp/format-position line col)
  (string-append (number->string (+ 1 line)) ":" (number->string (+ 1 col))))

;;; `grapheme-col-or-wire`: see CLAUDE.md's "Displayed value" sanctioned
;;; exception. `#f` falls back to `path:line`.
(define (lsp/location-display part)
  (let* ((path (path->display (car part)))
         (line (cadr part))
         (grapheme-col-or-wire (caddr part)))
    (string-append path ":"
      (if grapheme-col-or-wire
          (lsp/format-position line grapheme-col-or-wire)
          (number->string (+ 1 line))))))

(define (lsp/show-locations! locs)
  (show-drawer-list! (map lsp/location-display (lsp-locations->display-parts locs))
    (lambda (idx) (when idx (goto-location! (list-ref locs idx))))))
