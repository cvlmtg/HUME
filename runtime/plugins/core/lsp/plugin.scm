;;; core:lsp — plugin.scm (see README.md "File layout").

(require "lib.scm")
(require "registration.scm")
(require "servers.scm")
(require "hover.scm")
(require "goto.scm")
(require "diagnostics.scm")
(require "rename.scm")
(require "format.scm")
(require "actions.scm")
(require "sighelp.scm")
(require "completion.scm")
(require "inlay.scm")

(unless (member "core:stdlib" (declared-plugins))
  (error "core:lsp: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:lsp\")"))

;; ── Register installed servers ────────────────────────────────────────────────

(lsp/register-installed-servers!)

;; Default keybindings — see README.md "Keys".
(bind-key! 'normal "g d" "lsp-goto-definition")
(bind-key! 'normal "g D" "lsp-goto-declaration")
(bind-key! 'normal "g y" "lsp-goto-type-definition")
(bind-key! 'normal "g i" "lsp-goto-implementation")
(bind-key! 'normal "z r" "lsp-references")
(bind-key! 'normal "g r" "lsp-rename")
(bind-key! 'normal "z k" "lsp-hover")
(bind-key! 'normal "z a" "lsp-code-actions")
(bind-key! 'normal "g n" "goto-next-diagnostic")
(bind-key! 'normal "g p" "goto-prev-diagnostic")
(bind-key! 'insert "ctrl-space" "lsp-completion-trigger")
