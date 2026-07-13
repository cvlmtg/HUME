;;; HUME's PLUgin Manager

(require "plugins.scm")
(require "grammars.scm")
(require "servers.scm")

;; ── Load grammar source catalog ───────────────────────────────────────────────

(for-each
  plum/declare-grammar-source!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "grammar-sources.scm")
    read))

;; ── Load LSP server catalogs ──────────────────────────────────────────────────

(for-each
  plum/declare-lsp-server!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "lsp-servers.scm")
    read))

(for-each
  plum/declare-lsp-source!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "lsp-sources.scm")
    read))

;; ── Register installed grammars ────────────────────────────────────────────────
;;
;; LSP servers are not registered here — that's core:lsp's job (see
;; runtime/plugins/core/lsp/registration.scm). PLUM only installs them.

(plum/register-installed-grammars!)
