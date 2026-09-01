;;; core:lsp/lib.scm — shared helpers used by every feature file. See
;;; README.md "How it works" → "Shared helpers (lib.scm)".

(provide lsp/supports? lsp/supports-for-buffer? lsp/guard-capability lsp/report-error
         lsp/visible-lines lsp/show-locations! lsp/text-edit->tuple
         lsp/setup-trigger-chars! lsp/format-position lsp/cap-field lsp/cap-flag?)

;; ── Capability guard ────────────────────────────────────────────────────────

;;; `caps` hash present, advertises `cap-key`, and not explicitly `#f`.
(define (lsp/caps-has-cap? caps cap-key)
  (and caps
       (hash-contains? caps cap-key)
       (not (equal? (hash-ref caps cap-key) #f))))

;;; #t if the focused buffer's attached server advertises `cap-key`.
(define (lsp/supports? cap-key)
  (lsp/caps-has-cap? (lsp-capabilities #f) cap-key))

;;; Per-buffer variant, for hook callbacks whose buffer isn't necessarily
;;; the focused one. `#f` when `bid` has no attached server at all.
(define (lsp/supports-for-buffer? bid cap-key)
  (let ((server (lsp-server-for-buffer bid)))
    (and server (lsp/caps-has-cap? (lsp-capabilities server) cap-key))))

;;; Run `thunk` only if the focused buffer's server supports `cap-key`;
;;; otherwise report politely and skip.
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

;;; #t if the focused buffer's server advertises `cap-key`'s `field` as
;;; exactly `#t` (e.g. rangeFormatting's `rangesSupport`, codeAction's
;;; `resolveProvider`).
(define (lsp/cap-flag? cap-key field)
  (equal? (lsp/cap-field (lsp-capabilities #f) cap-key field #f) #t))

;; ── Popup dismissal ─────────────────────────────────────────────────────────
;; Shared by every feature using a popup (hover, sighelp, …).
(register-hook! 'on-mode-change (lambda (old-mode new-mode) (close-popup!)))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

;;; Wires up on-lsp-attach/on-lsp-detach/on-trigger-char for a trigger-char
;;; feature (completion, sighelp). `extra-chars` are always-registered chars
;;; beyond the server's own; `on-trigger` fires only for `source-name`'s own
;;; chars. See README for why this is keyed `(source, language)`.
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

;;; One `'error` log line for a callback error — `err` is either a
;;; {"code" "message"} hashmap or the bare string "timeout".
(define (lsp/report-error what err)
  (log! 'error
        (string-append "lsp " what ": "
                       (if (string? err) err (hash-ref err "message")))))

;;; A `TextEdit` hashmap `{range: {start, end}, newText}` -> the
;;; `((start-line . start-character) (end-line . end-character) text)` tuple
;;; shape `apply-text-edits!` expects. Only caller is format.scm.
(define (lsp/text-edit->tuple te)
  (let* ((range (hash-ref te "range"))
         (start (hash-ref range "start"))
         (end (hash-ref range "end")))
    (list (cons (hash-ref start "line") (hash-ref start "character"))
          (cons (hash-ref end "line") (hash-ref end "character"))
          (hash-ref te "newText"))))

;; ── Viewport ────────────────────────────────────────────────────────────────
;; `(viewport-range bid)` is a synchronous Rust builtin; see hover.scm's
;; popup threshold and inlay.scm's refresh trigger for its two callers.

;;; Number of lines currently visible in `bid`'s pane, or `#f` if `bid` isn't
;;; shown in any pane. `viewport-range` is 0-based end-exclusive, so the
;;; count is just the range's width — no `+ 1` needed.
(define (lsp/visible-lines bid)
  (let ((range (viewport-range bid)))
    (if range (- (cdr range) (car range)) #f)))

;; ── Location display + drawer ───────────────────────────────────────────────
;; Location-shape dispatch lives in Rust (hume_lsp::location::decode_location,
;; shared by goto-location! and lsp-locations->display-parts) — see README.

;;; "12:5" — 1-based line/column from 0-based `line`/`col`. The single
;;; formatter for every "L:C" HUME shows a user (`lsp/location-display`
;;; below, `:diagnostics`'s drawer rows).
(define (lsp/format-position line col)
  (string-append (number->string (+ 1 line)) ":" (number->string (+ 1 col))))

;;; "path/to/file.rs:12:5" — 1-based line/column, from one `(path line
;;; grapheme-col-or-wire)` entry (see CLAUDE.md's "Displayed value"
;;; sanctioned exception for what `grapheme-col-or-wire` means). `#f` falls
;;; back to `path:line`. `path->display` is the only formatting still done
;;; here (`~` collapse, UNC strip).
(define (lsp/location-display part)
  (let* ((path (path->display (car part)))
         (line (cadr part))
         (grapheme-col-or-wire (caddr part)))
    (string-append path ":"
      (if grapheme-col-or-wire
          (lsp/format-position line grapheme-col-or-wire)
          (number->string (+ 1 line))))))

;;; `locs`: a list of raw `Location`/`LocationLink` hashmaps, decoded once by
;;; `lsp-locations->display-parts`. Drawer rows are pre-formatted display
;;; strings.
(define (lsp/show-locations! locs)
  (show-drawer-list! (map lsp/location-display (lsp-locations->display-parts locs))
    (lambda (idx) (when idx (goto-location! (list-ref locs idx))))))
