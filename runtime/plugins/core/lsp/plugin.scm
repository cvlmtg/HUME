;;; core:lsp — LSP features (hover, goto, completions, diagnostics, rename,
;;; formatting, code actions, signature help, inlay hints) composed from the
;;; generic bridge and platform primitives (docs/LSP.md hub, Step 2/3).
;;;
;;; Depends on core:stdlib (F4's diagnostic navigation calls
;;; stdlib/cursor-char-index) — load it first.

(require "lib.scm")
(require "hover.scm")
(require "goto.scm")
(require "diagnostics.scm")
(require "rename.scm")
(require "format.scm")
(require "actions.scm")
(require "sighelp.scm")
