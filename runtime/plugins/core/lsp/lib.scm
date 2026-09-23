;;; core:lsp/lib.scm — shared helpers used by every feature file. See
;;; docs/architecture.md.

(provide lsp/supports? lsp/supports-for-buffer? lsp/guard-capability lsp/report-error
         lsp/visible-lines lsp/show-locations! lsp/text-edit->tuple
         lsp/setup-trigger-chars! lsp/format-position lsp/cap-field lsp/cap-flag?)

;; ── Capability guard ────────────────────────────────────────────────────────
;;; `caps`, throughout this section, is `#f` or the JSON handle
;;; `(lsp-capabilities …)` returns onto the server's wire `ServerCapabilities`.

(define (lsp/caps-has-cap? caps cap-key)
  (and caps
       (json-contains? caps cap-key)
       (not (equal? (json-ref caps cap-key) #f))))

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

;;; `field`'s value inside `cap-key`'s CapabilityOptions object (a scalar —
;;; a native value, never a container — `"resolveProvider"`/`"rangesSupport"`,
;;; the only two `field`s any caller reads), or `default` when `cap-key` is
;;; absent or is a bare boolean rather than an options object.
(define (lsp/cap-field caps cap-key field default)
  (if (and caps (json-contains? caps cap-key))
      (let ((cap (json-ref caps cap-key)))
        (if (and (json-object? cap) (json-contains? cap field))
            (json-ref cap field)
            default))
      default))

(define (lsp/cap-flag? cap-key field)
  (equal? (lsp/cap-field (lsp-capabilities #f) cap-key field #f) #t))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

;;; `on-trigger` is `#f` for a completion source registered under
;;; `source-name`, invoked directly by the editor against its own trigger
;;; chars (`completion-set-trigger-chars!`) — not through the shared,
;;; listener-agnostic `register-trigger-chars!`/`on-trigger-char` table,
;;; which serves every *other* trigger-char listener (signature help).
(define (lsp/setup-trigger-chars! cap-key source-name extra-chars on-trigger)
  (define (set-chars! server-name chars)
    (if on-trigger
        (register-trigger-chars! source-name server-name chars)
        (completion-set-trigger-chars! source-name server-name chars)))
  (register-hook! 'on-lsp-attach
    (lambda (bid server-name)
      (let ((caps (lsp-capabilities server-name)))
        (when (and caps (json-contains? caps cap-key))
          ;; "triggerCharacters" is a JSON array, unlike lsp/cap-field's
          ;; other (scalar) callers — unpacked to a Steel list of strings
          ;; here, at this one call site, rather than folding an
          ;; array-vs-scalar branch into lsp/cap-field's own contract.
          (let* ((cap (json-ref caps cap-key))
                 (trigger-chars (if (and (json-object? cap) (json-contains? cap "triggerCharacters"))
                                     (json-list (json-ref cap "triggerCharacters"))
                                     (list))))
            (set-chars! server-name (append extra-chars trigger-chars)))))))
  (register-hook! 'on-lsp-detach
    (lambda (bid server-name)
      (set-chars! server-name '())))
  (when on-trigger
    (register-hook! 'on-trigger-char
      (lambda (bid ch source)
        (when (equal? source source-name)
          (on-trigger bid ch))))))

(define (lsp/report-error what err)
  (log! 'error
        (string-append "lsp " what ": "
                       (if (string? err) err (hash-ref err "message")))))

;;; `TextEdit` JSON handle -> the tuple shape `apply-text-edits!` expects.
(define (lsp/text-edit->tuple te)
  (list (cons (json-ref te "range" "start" "line") (json-ref te "range" "start" "character"))
        (cons (json-ref te "range" "end" "line") (json-ref te "range" "end" "character"))
        (json-ref te "newText")))

;; ── Viewport ────────────────────────────────────────────────────────────────

;;; #f if `bid` isn't shown in a pane on the active tab (including a buffer
;;; visible only in a background tab).
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
