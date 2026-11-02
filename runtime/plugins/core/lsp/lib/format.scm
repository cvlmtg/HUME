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

;; `get-ranges` a thunk returning a list of LSP ranges, or #f for the whole
;; buffer; `done` a thunk called once the format ends, or #f. `quiet` skips the
;; notices for a buffer no server can format and for a buffer that needs no
;; change. See docs/features.md, "Range requests".
(define (lsp/format-document! pane get-ranges done quiet)
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
  (define route
    (lsp-route pane #:method (if get-ranges "textDocument/rangeFormatting" "textDocument/formatting")))
  (define (document) (hash-ref (lsp-document-params pane) "textDocument"))
  (cond
    ((hash-ref route 'err)
     (unless quiet
       (lsp/report-error! (if get-ranges "range formatting" "format") (hash-ref route 'err)))
     (finish!))
    ((not get-ranges)
     (lsp-request! pane "textDocument/formatting"
       (hash "textDocument" (document) "options" (lsp/format-options pane))
       on-answer
       #:allow-stale #t))
    (else
     (let ((server (car (hash-ref route 'servers)))
           (td (document))
           (ranges (get-ranges)))
       (cond
         ((null? ranges) (finish!))
         ((and (> (length ranges) 1)
               (lsp/cap-flag? (lsp-capability server #:method "textDocument/rangeFormatting")
                              "rangesSupport"))
          (lsp-request! pane "textDocument/rangesFormatting"
            (hash "textDocument" td "ranges" ranges "options" (lsp/format-options pane))
            on-answer
            #:to server
            #:allow-stale #t))
         ((> (length ranges) lsp/format-max-ranges)
          (log! 'info
                (string-append (number->string (length ranges))
                               " ranges exceeds format-max-ranges ("
                               (number->string lsp/format-max-ranges)
                               ") — nothing formatted"))
          (finish!))
         (else (lsp/format-fan-out! pane server td ranges fail! apply!)))))))

(define (lsp/format-source! pane)
  (cond
    ((selections-linewise? pane)
     (lsp/format-document! pane
                           (lambda () (hash-ref (lsp-linewise-ranges-params pane) "ranges"))
                           #f
                           #f))
    ((selections-charwise? pane) (lsp/format-document! pane #f #f #f))
    (else (log! 'info "mixed whole-line and partial selections — nothing formatted"))))

(define-command! "lsp/format-source"
  "Format the buffer via LSP — bind this to a key. Formats the whole-line selections, or the whole buffer when none are whole-line."
  (lambda (pane) (lsp/format-source! pane)))

(define-command! "lsp/format-document"
  "Format the whole buffer via LSP, or the LSP ranges given after `done` (which may be #f), then call `done`. Works on a pane or on a buffer no pane shows. For hooks: `(call! \"lsp/format-document\" pane done)` in an `on-before-buffer-save` handler."
  (lambda (pane done . ranges)
    (when (> (length ranges) 1)
      (error "lsp/format-document: takes pane, done and at most one ranges list"))
    (lsp/format-document! pane (if (null? ranges) #f (lambda () (car ranges))) done #t)))

(define-typed-command! "format-source"
  ":format-source — format the buffer via LSP from the command line."
  (lambda (pane) (lsp/format-source! pane)))
