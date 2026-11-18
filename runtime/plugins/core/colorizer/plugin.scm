;;; core:colorizer — plugin.scm. See README.md.

(require "lib/render.scm")

;; ── Config ────────────────────────────────────────────────────────────────────

(define colorizer/cfg (plugin-config))
(define colorizer/opts
  (colorizer/options
    (call! "stdlib/config-boolean" "core:colorizer" colorizer/cfg 'enabled #t)
    (call! "stdlib/config-enum" "core:colorizer" colorizer/cfg 'mode 'background
           '(background foreground))
    (call! "stdlib/config-list" "core:colorizer" colorizer/cfg 'named-languages
           '("css" "scss" "less"))
    (call! "stdlib/config-integer" "core:colorizer" colorizer/cfg 'max-render-chars 7100 1)))

;; ── Rendering ─────────────────────────────────────────────────────────────────

(define (colorizer/render! pane)
  (colorizer/render-buffer! pane colorizer/opts))

(define colorizer/render-soon!
  (debounce-by 50 colorizer/render! #:key (lambda (p . _) (buffer-key p))))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-viewport-change
  (lambda (pane range headers) (colorizer/render! pane)))

(register-hook! 'on-text-changed
  (lambda (pane changes) (colorizer/render-soon! pane)))

(register-hook! 'on-language-set
  (lambda (pane language) (colorizer/render! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane) (colorizer/remove-buffer! pane)))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define (colorizer/toggle! pane)
  (colorizer/track-buffer! pane colorizer/opts)
  (if (colorizer/toggle-enabled! pane)
      (colorizer/render! pane)
      (colorizer/clear! pane)))

(define colorizer/toggle-doc
  "Toggle painting color literals, such as `#ff0000`, with the color they name in the current buffer.")

(define-command! "colorizer/toggle" colorizer/toggle-doc colorizer/toggle!)

(define-typed-command! "toggle-colorizer" colorizer/toggle-doc colorizer/toggle!)
