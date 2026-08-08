;;; core:lsp/lib.scm — shared helpers used by every feature file.

(provide lsp/supports? lsp/supports-for-buffer? lsp/guard-capability lsp/report-error
         lsp/visible-lines lsp/show-locations! lsp/normalize-location lsp/text-edit->tuple
         lsp/utf16-offset->char-index
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
;;; `((start-line . start-col) (end-line . end-col) text)` tuple shape
;;; `apply-text-edits!` expects. Shared by completion and formatting.
(define (lsp/text-edit->tuple te)
  (let* ((range (hash-ref te "range"))
         (start (hash-ref range "start"))
         (end (hash-ref range "end")))
    (list (cons (hash-ref start "line") (hash-ref start "character"))
          (cons (hash-ref end "line") (hash-ref end "character"))
          (hash-ref te "newText"))))

;; ── UTF-16 offset conversion ────────────────────────────────────────────────
;; LSP wire positions are UTF-16 code-unit offsets; Steel strings index by
;; Unicode scalar value, which only diverges for astral-plane chars (a
;; surrogate pair, 2 units, but 1 Steel char). Only signature-help's
;; offset-form parameter labels expose a raw offset pair to Steel code.

;;; `offset`: a UTF-16 code-unit offset into `s` -> the char index
;;; `string-ref`/`substring` expect.
(define (lsp/utf16-offset->char-index s offset)
  (let loop ((i 0) (n (string-length s)) (units 0))
    (if (or (>= i n) (>= units offset))
        i
        (loop (+ i 1) n (+ units (if (>= (char->integer (string-ref s i)) #x10000) 2 1))))))

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
;; Everything here normalizes to `{uri, range}` once, at response ingress,
;; so nothing downstream needs its own Location/LocationLink shape dispatch.

;;; A raw `Location` or `LocationLink` hashmap -> the single `{uri, range}`
;;; shape every Steel-side consumer works with.
(define (lsp/normalize-location loc)
  (if (hash-contains? loc "uri")
      loc
      (hash "uri" (hash-ref loc "targetUri")
            "range" (if (hash-contains? loc "targetSelectionRange")
                        (hash-ref loc "targetSelectionRange")
                        (hash-ref loc "targetRange")))))

;;; "path/to/file.rs" stripped of the "file://" scheme prefix — good enough
;;; for display, not for parsing back into a URI (no percent-decoding, no
;;; UNC-share authority handling — see `hume_lsp::uri::uri_to_path` on the
;;; Rust side for that). A Windows drive-letter URI (`file:///C:/foo`)
;;; decodes to an extra leading '/' before the drive letter that a plain
;;; 7-char strip leaves in ("/C:/foo"); drop it so the result reads
;;; "C:/foo" like every other path this file displays.
(define (lsp/uri->display-path uri)
  (define (ascii-letter? c)
    (let ([n (char->integer c)])
      (or (and (>= n 65) (<= n 90)) (and (>= n 97) (<= n 122)))))
  (define (strip-drive-letter-slash s)
    (if (and (>= (string-length s) 3)
             (equal? (substring s 0 1) "/")
             (ascii-letter? (string-ref s 1))
             (equal? (substring s 2 3) ":"))
        (substring s 1 (string-length s))
        s))
  (if (and (>= (string-length uri) 7) (equal? (substring uri 0 7) "file://"))
      (path->display (strip-drive-letter-slash (substring uri 7 (string-length uri))))
      uri))

;;; "path/to/file.rs:12:5" — 1-based line/col, matching every other editor's
;;; location display convention (the wire values are 0-based). `loc` must
;;; already be normalized (`{uri, range}`).
(define (lsp/location-display loc)
  (let* ((uri (hash-ref loc "uri"))
         (start (hash-ref (hash-ref loc "range") "start")))
    (string-append (lsp/uri->display-path uri) ":"
                   (number->string (+ 1 (hash-ref start "line"))) ":"
                   (number->string (+ 1 (hash-ref start "character"))))))

;;; `locs`: a list of already-normalized `{uri, range}` hashmaps. Drawer rows
;;; are pre-formatted display strings.
(define (lsp/show-locations! locs)
  (show-drawer-list! (map lsp/location-display locs)
    (lambda (idx) (when idx (goto-location! (list-ref locs idx))))))
