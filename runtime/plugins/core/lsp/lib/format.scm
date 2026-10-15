;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting /
;;; rangesFormatting. See docs/features.md.

(require "helpers.scm")

(define lsp/format-max-ranges
  (call! "stdlib/config-integer" "core:lsp" (plugin-config) "format-max-ranges" 16 1))

(define (lsp/format-options pane)
  (hash "tabSize" (get-buffer-option pane "tab-width")
        "insertSpaces" (equal? (get-buffer-option pane "tab-style") 'soft)))

(define (lsp/format-edits res)
  (if (void? res) (list) (json-list res)))

(define (lsp/finish! done)
  (when done (done)))

(define (lsp/format-apply! pane gen edits done)
  (if (null? edits)
      (log! 'info "Already formatted")
      (with-handler
        (lambda (e) (log! 'error (string-append "lsp lsp-fmt: " (to-string e))))
        (apply-text-edits! pane edits #:expect-generation gen)))
  (lsp/finish! done))

(define (lsp/format-callback pane gen done)
  (lambda (err res)
    (if err
        (begin (lsp/report-error! "lsp-fmt" err) (lsp/finish! done))
        (lsp/format-apply! pane gen (lsp/format-edits res) done))))

(define (lsp/format-fan-out! pane server gen td ranges done)
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
                    (lsp/report-error! "lsp-fmt" err)
                    (lsp/finish! done))
                  (begin
                    (set-box! edits (append (unbox edits) (lsp/format-edits res)))
                    (set-box! pending (- (unbox pending) 1))
                    (when (= (unbox pending) 0)
                      (lsp/format-apply! pane gen (unbox edits) done))))))
          #:to server
          #:allow-stale #t))
      ranges)))

;; See docs/features.md, "Range requests".
(define (lsp/format-linewise! pane gen td ranges done)
  (let ((servers (lsp-servers pane #:method "textDocument/rangeFormatting"))
        (n (length ranges)))
    (lsp/with-servers servers "range formatting"
      (lambda (servers)
        (let ((server (car servers)))
          (cond
            ((and (> n 1) (lsp/cap-flag? (lsp-capability server #:method "textDocument/rangeFormatting") "rangesSupport"))
             (lsp-request! pane "textDocument/rangesFormatting"
               (hash "textDocument" td "ranges" ranges "options" (lsp/format-options pane))
               (lsp/format-callback pane gen done)
               #:to server
               #:allow-stale #t))
            ((> n lsp/format-max-ranges)
             (log! 'info
                   (string-append (number->string n)
                                   " ranges exceeds format-max-ranges ("
                                   (number->string lsp/format-max-ranges)
                                   ") — nothing formatted"))
             (lsp/finish! done))
            (else (lsp/format-fan-out! pane server gen td ranges done))))))
    (when (null? servers) (lsp/finish! done))))

;; `td` is the params' "textDocument"; `ranges` a list of LSP ranges, or #f.
(define (lsp/format-document! pane td ranges done)
  (let ((gen (buffer-generation pane)))
    (if ranges
        (lsp/format-linewise! pane gen td ranges done)
        (lsp-request! pane "textDocument/formatting"
          (hash "textDocument" td "options" (lsp/format-options pane))
          (lsp/format-callback pane gen done)
          #:allow-stale #t))))

(define (lsp/format-source! pane)
  (let ((rp (lsp-linewise-ranges-params pane)))
    (cond
      ((not rp) (log! 'info "buffer has no path — nothing to format"))
      ((selections-linewise? pane)
       (lsp/format-document! pane (hash-ref rp "textDocument") (hash-ref rp "ranges") #f))
      ((selections-charwise? pane)
       (lsp/format-document! pane (hash-ref rp "textDocument") #f #f))
      (else (log! 'info "mixed whole-line and partial selections — nothing formatted")))))

(define-command! "lsp-fmt"
  "Format the buffer via LSP — bind this to a key. Formats the whole-line selections, or the whole buffer when none are whole-line."
  (lambda (pane) (lsp/format-source! pane)))

(define-command! "lsp-fmt-document"
  "Format the whole buffer via LSP, or the LSP ranges given after `done` (which may be #f), then call `done`. Works on a pane or on a buffer no pane shows. For hooks: `(call! \"lsp-fmt-document\" pane done)` in an `on-before-buffer-save` handler."
  (lambda (pane done . ranges)
    (when (> (length ranges) 1)
      (error "lsp-fmt-document: takes pane, done and at most one ranges list"))
    (let ((params (lsp-document-params pane)))
      (if params
          (lsp/format-document! pane (hash-ref params "textDocument")
                                (if (null? ranges) #f (car ranges))
                                done)
          (begin
            (log! 'info "buffer has no path — nothing to format")
            (lsp/finish! done))))))

(define-typed-command! "format-source"
  ":format-source — format the buffer via LSP from the command line."
  (lambda (pane) (lsp/format-source! pane)))
