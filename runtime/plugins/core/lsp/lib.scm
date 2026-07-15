;;; core:lsp/lib.scm — shared helpers used by every feature file.

(provide lsp/supports? lsp/supports-for-buffer? lsp/guard-capability lsp/report-error
         lsp/visible-lines lsp/show-locations! lsp/text-edit->tuple
         lsp/utf16-offset->char-index
         lsp/setup-trigger-chars!)

;; ── Capability guard ────────────────────────────────────────────────────────

;;; Shared predicate: `caps` hash present, advertises `cap-key`, and not
;;; explicitly `#f` — the definition of "supported" both `lsp/supports?` and
;;; `lsp/supports-for-buffer?` check, differing only in how `caps` is obtained.
(define (lsp/caps-has-cap? caps cap-key)
  (and caps
       (hash-contains? caps cap-key)
       (not (equal? (hash-ref caps cap-key) #f))))

;;; #t if the focused buffer's attached server advertises `cap-key` (e.g.
;;; "hoverProvider") — missing or explicitly `#f` means unsupported.
(define (lsp/supports? cap-key)
  (lsp/caps-has-cap? (lsp-capabilities #f) cap-key))

;;; Per-buffer variant of `lsp/supports?` — needed wherever the buffer in
;;; question isn't necessarily the focused one (e.g. a hook argument fired
;;; for a background pane in a split). `#f` when `bid` has no attached
;;; server at all — never falls back to checking the focused buffer's
;;; capabilities instead.
(define (lsp/supports-for-buffer? bid cap-key)
  (let ((server (lsp-server-for-buffer bid)))
    (and server (lsp/caps-has-cap? (lsp-capabilities server) cap-key))))

;;; Run `thunk` only if the focused buffer's server supports `cap-key`;
;;; otherwise report politely and skip — every feature capability-checks
;;; before firing a request (hub primer).
(define (lsp/guard-capability cap-key thunk)
  (if (lsp/supports? cap-key)
      (thunk)
      (log! 'info
            (string-append "not supported by "
                           (let ((name (lsp-server-for-buffer (current-buffer))))
                             (if name name "server"))))))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

;;; Wires up the on-lsp-attach / on-lsp-detach / on-trigger-char trio a
;;; trigger-char-driven feature (completion, signature help) needs —
;;; `cap-key`/`source-name` name the capability and the
;;; `register-trigger-chars!` source; `extra-chars` are always-registered
;;; chars beyond whatever the server advertises (e.g. sighelp's dismiss
;;; ")"); `on-trigger` is `(lambda (bid ch) ...)`, called only for a `ch`
;;; registered under this feature's own `source-name`. No closure state:
;;; `register-trigger-chars!` is keyed `(source, language)` on the Rust side
;;; — `server-name` (the `on-lsp-attach`/`on-lsp-detach` arg) *is* the
;;; registered language — so a second language attaching under the same
;;; `source-name` gets its own entry instead of clobbering the first's.
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
;;; {"code" "message"} hashmap (protocol error) or the bare string
;;; "timeout" (deadline expiry, including a request orphaned by a server
;;; crash — there is no separate "server-crashed" err value).
(define (lsp/report-error what err)
  (log! 'error
        (string-append "lsp " what ": "
                       (if (string? err) err (hash-ref err "message")))))

;;; A `TextEdit` hashmap `{range: {start, end}, newText}` -> the
;;; `((start-line start-col) (end-line end-col) text)` tuple shape
;;; `apply-text-edits!` expects. Shared by completion (additionalTextEdits)
;;; and formatting.
(define (lsp/text-edit->tuple te)
  (let* ((range (hash-ref te "range"))
         (start (hash-ref range "start"))
         (end (hash-ref range "end")))
    (list (list (hash-ref start "line") (hash-ref start "character"))
          (list (hash-ref end "line") (hash-ref end "character"))
          (hash-ref te "newText"))))

;; ── UTF-16 offset conversion ────────────────────────────────────────────────
;; LSP wire positions are UTF-16 code-unit offsets (this client negotiates
;; `positionEncoding: utf-16`, its default). Steel strings index by Unicode
;; scalar value (`string-ref`/`substring`), which only diverges from UTF-16
;; units for astral-plane chars (>= U+10000 — a surrogate pair, 2 units, but
;; 1 Steel char). Signature-help's offset-form parameter labels are the one
;; place a raw UTF-16 offset pair reaches Steel code directly.

;;; `offset`: a UTF-16 code-unit offset into `s` -> the char index
;;; `string-ref`/`substring` expect. Stops at the first char whose
;;; cumulative unit count reaches `offset` — a surrogate-pair-splitting
;;; offset (never valid on the wire) lands on that char rather than
;;; between its two units.
(define (lsp/utf16-offset->char-index s offset)
  (let loop ((i 0) (n (string-length s)) (units 0))
    (if (or (>= i n) (>= units offset))
        i
        (loop (+ i 1) n (+ units (if (>= (char->integer (string-ref s i)) #x10000) 2 1))))))

;; ── Viewport ────────────────────────────────────────────────────────────────
;; `(viewport-range bid)` is a synchronous Rust builtin (pane geometry read
;; live off `EngineView`, not a Steel-side mirror) — see hover.scm's popup
;; threshold and inlay.scm's refresh trigger for its two callers.

;;; Number of lines currently visible in `bid`'s pane, or `#f` if `bid` isn't
;;; shown in any pane.
(define (lsp/visible-lines bid)
  (let ((range (viewport-range bid)))
    (if range (+ 1 (- (cadr range) (car range))) #f)))

;; ── Location display + drawer ───────────────────────────────────────────────
;; A `Location` is `{uri, range}`; a `LocationLink` is `{targetUri,
;; targetSelectionRange | targetRange, …}` — `goto-location!` accepts
;; either raw hashmap directly and does its own dual-shape extraction; these
;; mirror that extraction only for the human-readable display string.

(define (lsp/location-uri loc)
  (if (hash-contains? loc "uri") (hash-ref loc "uri") (hash-ref loc "targetUri")))

(define (lsp/location-start loc)
  (let ((range (if (hash-contains? loc "range")
                    (hash-ref loc "range")
                    (if (hash-contains? loc "targetSelectionRange")
                        (hash-ref loc "targetSelectionRange")
                        (hash-ref loc "targetRange")))))
    (hash-ref range "start")))

;;; "path/to/file.rs" stripped of the "file://" scheme prefix — good enough
;;; for display, not for parsing back into a URI.
(define (lsp/uri->display-path uri)
  (if (and (>= (string-length uri) 7) (equal? (substring uri 0 7) "file://"))
      (substring uri 7 (string-length uri))
      uri))

;;; "path/to/file.rs:12:5" — 1-based line/col, matching every other editor's
;;; location display convention (the wire values are 0-based).
(define (lsp/location-display loc)
  (let* ((uri (lsp/location-uri loc))
         (start (lsp/location-start loc)))
    (string-append (lsp/uri->display-path uri) ":"
                   (number->string (+ 1 (hash-ref start "line"))) ":"
                   (number->string (+ 1 (hash-ref start "character"))))))

;;; `locs`: a list of raw Location/LocationLink hashmaps (mixed shapes OK —
;;; each row's own on-select jump uses the original hashmap, not a
;;; re-derived one). Drawer rows are pre-formatted display strings;
;;; `goto-location!` handles the shape dispatch, so this never touches wire
;;; positions itself.
(define (lsp/show-locations! locs)
  (show-drawer-list! (map lsp/location-display locs)
    (lambda (idx) (when idx (goto-location! (list-ref locs idx))))))
