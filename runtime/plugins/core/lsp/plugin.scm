;;; core:lsp — plugin.scm (see docs/architecture.md).

(require "lib/helpers.scm")
(require "lib/status.scm")
(require "lib/hover.scm")
(require "lib/goto.scm")
(require "lib/diagnostics.scm")
(require "lib/rename.scm")
(require "lib/format.scm")
(require "lib/actions.scm")
(require "lib/sighelp.scm")
(require "lib/completion.scm")
(require "lib/inlay.scm")

(unless (member "core:stdlib" (declared-plugins))
  (error "core:lsp: requires core:stdlib — add (load-plugin! \"core:stdlib\") before (load-plugin! \"core:lsp\")"))

;; ── Default keybindings ───────────────────────────────────────────────────────

(bind-key! 'normal "g d" "lsp-goto-definition")
(bind-key! 'normal "g D" "lsp-goto-declaration")
(bind-key! 'normal "g y" "lsp-goto-type-definition")
(bind-key! 'normal "g i" "lsp-goto-implementation")
(bind-key! 'normal "z r" "lsp-references")
(bind-key! 'normal "G R" "lsp-rename")
(bind-key! 'normal "K" "lsp-hover")
(bind-key! 'normal "z a" "lsp-code-actions")
(bind-key! 'normal "g n" "goto-next-diagnostic")
(bind-key! 'normal "g p" "goto-prev-diagnostic")
