;;; core:markdown — plugin.scm. See docs/architecture.md.

(require "lib/state.scm")
(require "lib/render.scm")
(require "lib/text.scm")
(require "lib/links.scm")
(require "lib/paths.scm")

;; ── Config ────────────────────────────────────────────────────────────────────

(define md-preview/cfg (plugin-config))
(define md-preview/enabled-default
  (call! "stdlib/config-boolean" "core:markdown" md-preview/cfg 'preview #t))
(define md-preview/rule-width
  (call! "stdlib/config-integer" "core:markdown" md-preview/cfg 'rule-width 40 1))

;; ── Rendering ─────────────────────────────────────────────────────────────────

(define (md-preview/track! pane)
  (md-preview/track-buffer! pane md-preview/enabled-default))

(define (md-preview/render! pane)
  (md-preview/render-buffer! pane md-preview/rule-width))

(define md-preview/render-soon!
  (debounce-by 50 md-preview/render! #:key (lambda (p . _) (buffer-key p))))

(define (md-preview/warn-missing-grammars!)
  (for-each
    (lambda (name)
      (unless (language-has-grammar? name)
        (md-preview/warn-once! name
          (string-append "markdown: no " name " grammar, so its syntax shows raw;"
                         " run :plum-install-grammar " name))))
    '("markdown" "markdown.inline")))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (pane) (md-preview/warn-missing-grammars!)))

(register-hook! 'on-viewport-change
  (lambda (pane range headers) (md-preview/render! pane)))

(register-hook! 'on-syntax-changed
  (lambda (pane)
    (md-preview/track! pane)
    (md-preview/note-tree! pane)
    (md-preview/render-soon! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane)
    (md-preview/remove-buffer! pane)
    (when (buffer-live? pane)
      (md-preview/clear! pane))))

;; ── Links ─────────────────────────────────────────────────────────────────────

(register-hook! 'on-paste
  (lambda (pane pasted previous) (md-links/paste-link pasted previous)))

(register-completion-source! "markdown:paths" md-paths/complete!
  #:target 'buffer #:priority 20 #:token-chars "/.-")

(set-completion-triggers! "markdown" "markdown:paths" (list #\( #\/))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define (md-preview/toggle! pane)
  (unless (member (buffer-key pane) (map buffer-key (buffers)))
    (error "markdown: the current buffer is not a Markdown buffer"))
  (md-preview/track! pane)
  (let ([on? (md-preview/toggle-enabled! pane)])
    (if on?
        (md-preview/render! pane)
        (md-preview/clear! pane))
    (log! 'info (if on? "markdown preview on" "markdown preview off"))))

(define md-preview/toggle-doc
  "Toggle the Markdown live preview for the current buffer, which draws Markdown without its syntax.")

(define-command! "markdown/toggle-preview" md-preview/toggle-doc md-preview/toggle!)

(define-typed-command! "toggle-markdown-preview" md-preview/toggle-doc md-preview/toggle!)

(define (md-preview/format-text! text parsed)
  (unless (string? text)
    (error "markdown: format-text/markdown takes a text and its parsed tree"))
  (md-preview/format-text text parsed md-preview/rule-width))

(define-command! "format-text/markdown"
  "Given a Markdown text and its parsed tree, the conceals, line backgrounds and highlights that draw the text without its syntax. The editor calls it for every Markdown popup and the completion documentation box."
  md-preview/format-text!)
