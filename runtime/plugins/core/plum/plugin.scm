;;; HUME's PLUgin Manager — manages ordinary plugins and grammars. LSP server
;;; install/uninstall/registration is core:lsp's own responsibility (see
;;; runtime/plugins/core/lsp/servers.scm and registration.scm).

(require "plugins.scm")
(require "grammars.scm")

;; ── Load grammar source catalog ───────────────────────────────────────────────

(for-each
  plum/declare-grammar-source!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "grammar-sources.scm")
    read))

;; ── Register installed grammars ────────────────────────────────────────────────

(plum/register-installed-grammars!)
