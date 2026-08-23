;;; core:lsp/lib.scm — shared helpers used by every feature file.

(provide lsp/supports? lsp/supports-for-buffer? lsp/guard-capability lsp/report-error
         lsp/visible-lines lsp/show-locations! lsp/text-edit->tuple
         lsp/setup-trigger-chars!)

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

;; ── Popup dismissal ─────────────────────────────────────────────────────────
;; Shared popup widget — one registration for every feature using it
;; (hover, sighelp, …); harmless no-op when nothing is open.
(register-hook! 'on-mode-change (lambda (old-mode new-mode) (close-popup!)))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

;;; Wires up on-lsp-attach/on-lsp-detach/on-trigger-char for a trigger-char
;;; feature (completion, sighelp). `extra-chars` are always-registered chars
;;; beyond the server's own; `on-trigger` fires only for `source-name`'s own
;;; chars. Keyed `(source, language)` on the Rust side, so a second language
;;; attaching under the same `source-name` gets its own entry, not a clobber.
(define (lsp/setup-trigger-chars! cap-key source-name extra-chars on-trigger)
  (register-hook! 'on-lsp-attach
    (lambda (bid server-name)
      (let ((caps (lsp-capabilities server-name)))
        (when (and caps (hash-contains? caps cap-key))
          (let* ((provider (hash-ref caps cap-key))
                 (triggers (if (hash-contains? provider "triggerCharacters")
                               (hash-ref provider "triggerCharacters")
                               (list))))
            (register-trigger-chars! source-name server-name (append extra-chars triggers)))))))
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
;;; shape `apply-text-edits!` expects. Shared by completion and formatting.
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
;; A raw Location/LocationLink hashmap's `{uri, range}`-vs-`{targetUri,
;; targetRange}` shape dispatch lives in one place now:
;; `hume_lsp::location::decode_location`, shared by `goto-location!` (the
;; jump) and `lsp-locations->display-parts` (the drawer row) — nothing here
;; reads a location's wire fields directly any more.

;;; "path/to/file.rs:12:5" — 1-based line/column. `part` is one
;;; `(path line col)` entry from `lsp-locations->display-parts`: for a target
;;; with an open buffer, `col` is an exact grapheme column (or `#f` past the
;;; buffer's last line); for a target with no open buffer it's the
;;; location's own raw wire `character` instead — the one place HUME renders
;;; a wire unit directly, since resolving it exactly would mean reading a
;;; file the user may never open. Either way, `#f` falls back to `path:line`.
;;;
;;; `path->display` is the only formatting still done here (`~` collapse,
;;; UNC strip) — URI decoding and the line/column themselves come from the
;;; builtin, which read them out of the same location this row is naming.
(define (lsp/location-display part)
  (let* ((prefix (string-append (path->display (car part)) ":"
                                (number->string (+ 1 (cadr part)))))
         (grapheme-col (caddr part)))
    (if grapheme-col
        (string-append prefix ":" (number->string (+ 1 grapheme-col)))
        prefix)))

;;; `locs`: a list of raw `Location`/`LocationLink` hashmaps, decoded once by
;;; `lsp-locations->display-parts`. Drawer rows are pre-formatted display
;;; strings.
(define (lsp/show-locations! locs)
  (show-drawer-list! (map lsp/location-display (lsp-locations->display-parts locs))
    (lambda (idx) (when idx (goto-location! (list-ref locs idx))))))
