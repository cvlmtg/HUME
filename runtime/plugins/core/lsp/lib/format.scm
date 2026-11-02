;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting /
;;; rangesFormatting. See docs/features.md.

(require "helpers.scm")

(define lsp/format-max-ranges
  (call! "stdlib/config-integer" "core:lsp" (plugin-config) 'format-max-ranges 16 1))

(define (lsp/format-options pane)
  (hash "tabSize" (get-buffer-option pane "indent-width")
        "insertSpaces" (equal? (get-buffer-option pane "indent-style") "soft")))

(define (lsp/format-edits res)
  (if (void? res) (list) (json-list res)))

;; Sends one `rangeFormatting` request per range to `server` and hands
;; `apply!` every answer's edits once all have answered, or `fail!` the first
;; error.
(define (lsp/format-fan-out! pane server td ranges fail! apply!)
  (let ((pending (box (length ranges)))
        (edits (box (list)))
        (aborted (box #f))
        (opts (lsp/format-options pane)))
    (for-each
      (lambda (range)
        (lsp-request! pane "textDocument/rangeFormatting"
          (hash "textDocument" td "range" range "options" opts)
          (lambda (err res)
            (unless (unbox aborted)
              (if err
                  (begin
                    (set-box! aborted #t)
                    (fail! err))
                  (begin
                    (set-box! edits (cons (lsp/format-edits res) (unbox edits)))
                    (set-box! pending (- (unbox pending) 1))
                    (when (= (unbox pending) 0)
                      (apply! (call! "stdlib/concat" (reverse (unbox edits)))))))))
          #:to server
          #:allow-stale #t))
      ranges)))

;; `td` is the params' "textDocument"; `ranges` a list of LSP ranges, or #f;
;; `done` a thunk called once the format ends, or #f. `quiet` skips the
;; notices for a buffer no server can format and for a buffer that needs no
;; change. See docs/features.md, "Range requests".
(define (lsp/format-document! pane td ranges done quiet)
  (define gen (buffer-generation pane))
  (define (finish!) (when done (done)))
  (define (fail! err)
    (lsp/report-error! "format" err)
    (finish!))
  (define (apply! edits)
    (if (null? edits)
        (unless quiet (log! 'info "Already formatted"))
        (with-handler
          (lambda (e) (log! 'error (string-append "lsp format: " (to-string e))))
          (apply-text-edits! pane edits #:expect-generation gen)))
    (finish!))
  (define (on-answer err res)
    (if err (fail! err) (apply! (lsp/format-edits res))))
  (define servers
    (if ranges (lsp-servers pane #:method "textDocument/rangeFormatting") '()))
  (cond
    ((not ranges)
     (lsp-request! pane "textDocument/formatting"
       (hash "textDocument" td "options" (lsp/format-options pane))
       on-answer
       #:allow-stale #t
       #:unavailable (if quiet 'empty 'error)))
    ((null? ranges) (finish!))
    ((null? servers)
     (unless quiet (lsp/report-unsupported "range formatting"))
     (finish!))
    ((and (> (length ranges) 1)
          (lsp/cap-flag? (lsp-capability (car servers) #:method "textDocument/rangeFormatting")
                         "rangesSupport"))
     (lsp-request! pane "textDocument/rangesFormatting"
       (hash "textDocument" td "ranges" ranges "options" (lsp/format-options pane))
       on-answer
       #:to (car servers)
       #:allow-stale #t))
    ((> (length ranges) lsp/format-max-ranges)
     (log! 'info
           (string-append (number->string (length ranges))
                          " ranges exceeds format-max-ranges ("
                          (number->string lsp/format-max-ranges)
                          ") — nothing formatted"))
     (finish!))
    (else (lsp/format-fan-out! pane (car servers) td ranges fail! apply!))))

(define (lsp/format-source! pane)
  (let ((rp (lsp-linewise-ranges-params pane)))
    (cond
      ((not rp) (log! 'info "buffer has no path — nothing to format"))
      ((selections-linewise? pane)
       (lsp/format-document! pane (hash-ref rp "textDocument") (hash-ref rp "ranges") #f #f))
      ((selections-charwise? pane)
       (lsp/format-document! pane (hash-ref rp "textDocument") #f #f #f))
      (else (log! 'info "mixed whole-line and partial selections — nothing formatted")))))

(define-command! "lsp/format-source"
  "Format the buffer via LSP — bind this to a key. Formats the whole-line selections, or the whole buffer when none are whole-line."
  (lambda (pane) (lsp/format-source! pane)))

(define-command! "lsp/format-document"
  "Format the whole buffer via LSP, or the LSP ranges given after `done` (which may be #f), then call `done`. Works on a pane or on a buffer no pane shows. For hooks: `(call! \"lsp/format-document\" pane done)` in an `on-before-buffer-save` handler."
  (lambda (pane done . ranges)
    (when (> (length ranges) 1)
      (error "lsp/format-document: takes pane, done and at most one ranges list"))
    (let ((params (lsp-document-params pane)))
      (if params
          (lsp/format-document! pane (hash-ref params "textDocument")
                                (if (null? ranges) #f (car ranges))
                                done
                                #t)
          (begin
            (log! 'info "buffer has no path — nothing to format")
            (when done (done)))))))

(define-typed-command! "format-source"
  ":format-source — format the buffer via LSP from the command line."
  (lambda (pane) (lsp/format-source! pane)))
