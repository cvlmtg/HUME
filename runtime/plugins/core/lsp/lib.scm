;;; core:lsp/lib.scm — shared helpers used by every feature file.

(provide lsp/supports? lsp/guard-capability lsp/report-error lsp/visible-lines
         lsp/uri->display-path lsp/show-locations! lsp/text-edit->tuple lsp/viewport-range)

;; ── Capability guard ────────────────────────────────────────────────────────

;;; #t if the focused buffer's attached server advertises `cap-key` (e.g.
;;; "hoverProvider") — missing or explicitly `#f` means unsupported.
(define (lsp/supports? cap-key)
  (let ((caps (lsp-capabilities #f)))
    (and caps
         (hash-contains? caps cap-key)
         (not (equal? (hash-ref caps cap-key) #f)))))

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

;;; One `'error` log line for a B2 callback error — `err` is either a
;;; {"code" "message"} hashmap (protocol error) or a bare string
;;; ("timeout"/"server-crashed").
(define (lsp/report-error what err)
  (log! 'error
        (string-append "lsp " what ": "
                       (if (string? err) err (hash-ref err "message")))))

;;; A `TextEdit` hashmap `{range: {start, end}, newText}` -> the
;;; `((start-line start-col) (end-line end-col) text)` tuple shape
;;; `apply-text-edits!` expects. Shared by F3 (additionalTextEdits) and
;;; F8 (formatting).
(define (lsp/text-edit->tuple te)
  (let* ((range (hash-ref te "range"))
         (start (hash-ref range "start"))
         (end (hash-ref range "end")))
    (list (list (hash-ref start "line") (hash-ref start "character"))
          (list (hash-ref end "line") (hash-ref end "character"))
          (hash-ref te "newText"))))

;; ── Viewport tracker ────────────────────────────────────────────────────────
;; No pane-geometry builtin exists — on-viewport-change (B7) is the only
;; Steel-visible viewport source. Buffer-id SteelVals are NOT `equal?` across
;; separate wrappings of the same underlying id (Arc-pointer equality), so
;; per-buffer state is an assoc-list searched with `buffer-id=?`, never a
;; hashmap keyed directly by a bid.

(define *lsp-viewports* '())  ; list of (bid first last)

(register-hook! 'on-viewport-change
  (lambda (bid first last)
    (set! *lsp-viewports*
          (cons (list bid first last)
                (filter (lambda (e) (not (buffer-id=? (list-ref e 0) bid)))
                        *lsp-viewports*)))))

;;; `(first last)` visible line pair for `bid` as of the last
;;; on-viewport-change fire, or `#f` before the first event for it.
(define (lsp/viewport-range bid)
  (let ((matches (filter (lambda (e) (buffer-id=? (list-ref e 0) bid)) *lsp-viewports*)))
    (if (null? matches)
        #f
        (let ((entry (car matches)))
          (list (list-ref entry 1) (list-ref entry 2))))))

;;; Number of lines visible in `bid`'s pane as of the last on-viewport-change
;;; fire, or `#f` before the first event for that buffer.
(define (lsp/visible-lines bid)
  (let ((range (lsp/viewport-range bid)))
    (if range (+ 1 (- (cadr range) (car range))) #f)))

;; ── Location display + drawer ───────────────────────────────────────────────
;; A `Location` is `{uri, range}`; a `LocationLink` is `{targetUri,
;; targetSelectionRange | targetRange, …}` — `goto-location!` (B6) accepts
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
;;; re-derived one). Drawer rows are pre-formatted display strings (U6);
;;; `goto-location!` handles the shape dispatch, so this never touches wire
;;; positions itself.
(define (lsp/show-locations! locs)
  (show-drawer-list! (map lsp/location-display locs)
    (lambda (idx) (when idx (goto-location! (list-ref locs idx))))))
