;;; core:lsp/sighelp.scm — F7: textDocument/signatureHelp.
;;;
;;; Dismiss on mode change (leaving Insert, entering Command, …) is already
;;; handled by hover.scm's `on-mode-change` handler — `show-popup!`/
;;; `close-popup!` is one shared widget (U4), so that single registration
;;; covers every feature using it, including this one. No separate
;;; registration here.

(require "lib.scm")

;;; A `SignatureHelp.parameters[].label` is either a plain string, or a
;;; `[start, end)` UTF-16 offset pair into the *signature's own* label —
;;; both forms resolve to the same parameter text.
(define (lsp/param-text sig-label param)
  (let ((param-label (hash-ref param "label")))
    (if (string? param-label)
        param-label
        (substring sig-label (car param-label) (cadr param-label)))))

;;; The chosen signature's label, plus the active parameter's own text
;;; marked with `⟨…⟩` on a second line (no styling API in `show-popup!`
;;; v1). No active parameter, or no parameters at all, ⇒ bare label.
;;; `active-idx` is clamped into range rather than trusted verbatim.
(define (lsp/sighelp-text sig active-idx)
  (let* ((label (hash-ref sig "label"))
         (params (if (hash-contains? sig "parameters") (hash-ref sig "parameters") (list))))
    (if (or (not active-idx) (null? params))
        label
        (let* ((idx (max 0 (min active-idx (- (length params) 1))))
               (text (lsp/param-text label (list-ref params idx))))
          (string-append label "\n⟨" text "⟩")))))

;;; `res`: a `SignatureHelp` — `signatures: []` is spec-valid ("nothing to
;;; show"), same as a null/void response at the call site.
(define (lsp/show-sighelp res)
  (let ((sigs (hash-ref res "signatures")))
    (if (null? sigs)
        (close-popup!)
        (let* ((active-sig-idx (if (hash-contains? res "activeSignature") (hash-ref res "activeSignature") 0))
               (idx (max 0 (min active-sig-idx (- (length sigs) 1))))
               (sig (list-ref sigs idx))
               (active-param-idx (if (hash-contains? res "activeParameter") (hash-ref res "activeParameter") #f)))
          (show-popup! (lsp/sighelp-text sig active-param-idx))))))

(define lsp/sighelp-request
  (debounce 150
    (lambda (bid)
      (lsp-request #f "textDocument/signatureHelp" (lsp-position-params bid)
        (lambda (err res)
          (cond
            (err (lsp/report-error "signature help" err) (close-popup!))
            ((void? res) (close-popup!))
            (else (lsp/show-sighelp res))))))))

;;; The char set this feature reacts to — `on-trigger-char` fires for
;;; *every* registered source's chars (union semantics, B10a), so the
;;; handler must filter to its own set rather than reacting to any char.
(define *sighelp-chars* '())

(register-hook! 'on-lsp-attach
  (lambda (bid server-name)
    (let ((caps (lsp-capabilities server-name)))
      (when (and caps (hash-contains? caps "signatureHelpProvider"))
        (let* ((sh (hash-ref caps "signatureHelpProvider"))
               (triggers (if (hash-contains? sh "triggerCharacters") (hash-ref sh "triggerCharacters") (list)))
               ;; ")" is a dismiss trigger, not a request trigger — still
               ;; needs registering or it never reaches Insert-mode text.
               (chars (cons ")" triggers)))
          (set! *sighelp-chars* chars)
          (register-trigger-chars! "lsp-sighelp" chars))))))

;;; Same global-not-per-server caveat as completion.scm's `on-lsp-detach` —
;;; `register-trigger-chars!` has no scoping narrower than the source name.
(register-hook! 'on-lsp-detach
  (lambda (bid server-name)
    (set! *sighelp-chars* '())
    (register-trigger-chars! "lsp-sighelp" '())))

(register-hook! 'on-trigger-char
  (lambda (bid ch)
    (when (member ch *sighelp-chars*)
      (if (equal? ch ")")
          (close-popup!)
          ;; Unlike completion.scm's trigger path, this used to call
          ;; lsp/sighelp-request directly with no capability guard — a
          ;; stale trigger char left registered past detach (or a server
          ;; that never advertised signatureHelpProvider in the first
          ;; place) hit lsp-request's server-resolution failure and logged
          ;; an Error on every matching keystroke instead of a polite skip.
          (lsp/guard-capability "signatureHelpProvider"
            (lambda () (lsp/sighelp-request bid)))))))
