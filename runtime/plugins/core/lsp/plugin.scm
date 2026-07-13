;;; core:lsp — LSP features (hover, goto, completions, diagnostics, rename,
;;; formatting, code actions, signature help, inlay hints) composed from the
;;; generic bridge and platform primitives. Also owns LSP server
;;; registration: loading this plugin registers any PLUM-installed server
;;; found on disk (see registration.scm) — PLUM only downloads servers, it
;;; never registers them.
;;;
;;; Depends on core:stdlib (diagnostic navigation calls
;;; stdlib/cursor-char-index) — load it first.

(require "lib.scm")
(require "registration.scm")
(require "hover.scm")
(require "goto.scm")
(require "diagnostics.scm")
(require "rename.scm")
(require "format.scm")
(require "actions.scm")
(require "sighelp.scm")
(require "completion.scm")
(require "inlay.scm")

;; ── Register installed servers ────────────────────────────────────────────────
;;
;; Passive: reads on-disk receipts written by PLUM's install pipeline, no
;; subprocess, no network. Runs here at load time, or later at lazy
;; activation — see registration.scm's `lsp/register-installed-servers!` doc
;; comment and docs/LSP-INSTALL.md "Registration model".

(lsp/register-installed-servers!)

;; Default keybindings — goto trie (`g …`), free against
;; keymap/defaults.rs (only g/e/h/l/s taken). No collisions to document.
;; `lsp-fmt`, `diagnostics`, and `lsp-completion-trigger` (already bound to
;; Ctrl+Space, see keymap/defaults.rs) stay typed-command/pre-bound only.
(bind-key! 'normal "g d" "lsp-goto-definition")
(bind-key! 'normal "g D" "lsp-goto-declaration")
(bind-key! 'normal "g y" "lsp-goto-type-definition")
(bind-key! 'normal "g i" "lsp-goto-implementation")
(bind-key! 'normal "g r" "lsp-references")
(bind-key! 'normal "g R" "lsp-rename")
(bind-key! 'normal "g k" "lsp-hover")
(bind-key! 'normal "g a" "lsp-code-actions")
(bind-key! 'normal "g n" "goto-next-diagnostic")
(bind-key! 'normal "g p" "goto-prev-diagnostic")
