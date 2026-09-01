;;; core:lsp/format.scm — textDocument/formatting / rangeFormatting /
;;; rangesFormatting. Format-on-save is NOT wired by default — v1 is manual
;;; `:lsp-fmt` only. To opt in, uncomment:
;; (register-hook! 'on-buffer-save (lambda (bid) (call! "lsp-fmt")))

(require "lib.scm")

(define (lsp/format-options)
  (hash "tabSize" (get-option "tab-width")
        "insertSpaces" (equal? (get-option "tab-style") "soft")))

;;; LSP 3.18's `textDocument/rangesFormatting`, gated on
;;; `documentRangeFormattingProvider.rangesSupport` — the field
;;; `lsp-capabilities` can only see because `LspClient` keeps the server's
;;; raw wire JSON alongside its typed decode (see `caps_json`'s doc comment
;;; in `hume-lsp/src/client.rs`).
(define (lsp/ranges-support?)
  (equal? (lsp/cap-field (lsp-capabilities #f) "documentRangeFormattingProvider"
                          "rangesSupport" #f)
          #t))

;;; A `textDocument/formatting`/`rangeFormatting`/`rangesFormatting` result
;;; is `TextEdit[]`, or `null`/void when the server found nothing to
;;; change — normalized to a (possibly empty) list of `apply-text-edits!`
;;; tuples so every request shape, and the fan-out accumulator below, share
;;; one reading.
(define (lsp/format-edits res)
  (if (or (void? res) (null? res))
      (list)
      (map lsp/text-edit->tuple res)))

;;; Applies `edits` as one transaction, or reports "already formatted" when
;;; there's nothing to apply — the shared tail for every `:lsp-fmt` request
;;; shape.
(define (lsp/format-apply! bid gen edits)
  (if (null? edits)
      (log! 'info "Already formatted")
      (apply-text-edits! bid edits #:expect-generation gen)))

(define (lsp/format-callback bid gen)
  (lambda (err res)
    (if err
        (lsp/report-error "lsp-fmt" err)
        (lsp/format-apply! bid gen (lsp/format-edits res)))))

;;; One `rangeFormatting` per range, all filed in a single eval so they
;;; share one buffer generation, with every edit applied in one
;;; `apply-text-edits!` once the last response lands. Applying each
;;; response as it arrives would bump the generation and make the next one
;;; reject, and would leave N undo steps behind one `:lsp-fmt`. A response
;;; dropped as stale never calls back, so an edit landing mid-flight
;;; abandons the whole format — which is what `#:expect-generation` would
;;; have done to it anyway. Any one error response aborts the whole set —
;;; no partial format.
(define (lsp/format-fan-out! bid gen td ranges)
  (let ((pending (box (length ranges)))
        (edits (box (list)))
        (aborted (box #f)))
    (for-each
      (lambda (range)
        (lsp-request #f "textDocument/rangeFormatting"
          (hash "textDocument" td "range" range "options" (lsp/format-options))
          (lambda (err res)
            (unless (unbox aborted)
              (if err
                  (begin
                    (set-box! aborted #t)
                    (lsp/report-error "lsp-fmt" err))
                  (begin
                    (set-box! edits (append (unbox edits) (lsp/format-edits res)))
                    (set-box! pending (- (unbox pending) 1))
                    (when (= (unbox pending) 0)
                      (lsp/format-apply! bid gen (unbox edits)))))))))
      ranges)))

;;; Dispatch once every selection in `bid` is confirmed linewise: one
;;; `rangesFormatting` request when the server supports it, one
;;; `rangeFormatting` on the primary selection (+ a warning) past
;;; `lsp.format-max-ranges`, else a fan-out — one `rangeFormatting` per
;;; range. A single range always lands in the fan-out branch, sending
;;; exactly one request — the same wire shape a lone linewise selection
;;; sent before this file existed.
(define (lsp/format-linewise! bid gen td ranges)
  (lsp/guard-capability "documentRangeFormattingProvider"
    (lambda ()
      (cond
        ((and (> (length ranges) 1) (lsp/ranges-support?))
         (lsp-request #f "textDocument/rangesFormatting"
           (hash "textDocument" td "ranges" ranges "options" (lsp/format-options))
           (lsp/format-callback bid gen)))
        ((> (length ranges) (get-option "lsp.format-max-ranges"))
         (log! 'warn
               (string-append (number->string (length ranges))
                               " ranges exceeds lsp.format-max-ranges — "
                               "formatting only the primary selection"))
         (lsp-request #f "textDocument/rangeFormatting"
           (hash-insert (lsp-primary-range-params bid) "options" (lsp/format-options))
           (lsp/format-callback bid gen)))
        (else (lsp/format-fan-out! bid gen td ranges))))))

(define-command! "lsp-fmt"
  ":lsp-fmt — format the buffer via LSP: the selected lines when every selection spans one or more complete lines, the whole buffer otherwise, or nothing (with a warning) for a mix of the two."
  (lambda ()
    (let* ((bid (current-buffer))
           (rp (lsp-linewise-ranges-params bid)))
      (if (not rp)
          ;; No path or no attached server — `lsp-server-for-buffer`
          ;; distinguishes the two, since a capability guard can't: without
          ;; a server there's no `lsp-capabilities` to check in the first
          ;; place.
          (log! 'info (if (lsp-server-for-buffer bid)
                           "buffer has no path — nothing to format"
                           "no LSP server attached to this buffer"))
          (let* ((td (hash-ref rp "textDocument"))
                 (ranges (hash-ref rp "ranges"))
                 (gen (buffer-generation bid)))
            (cond
              ((not (selections-linewise? bid))
               (if (null? ranges)
                   (lsp/guard-capability "documentFormattingProvider"
                     (lambda ()
                       (lsp-request #f "textDocument/formatting"
                         (hash "textDocument" td "options" (lsp/format-options))
                         (lsp/format-callback bid gen))))
                   (log! 'warn "mixed whole-line and partial selections — nothing formatted")))
              (else (lsp/format-linewise! bid gen td ranges))))))))
