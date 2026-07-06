;;; HUME's PLUgin Manager

(require "plugins.scm")
(require "grammars.scm")

;; ── Load grammar source catalog ───────────────────────────────────────────────

(for-each
  plum/declare-grammar-source!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "grammar-sources.scm")
    read))

;; ── Register installed grammars + queue missing ones ─────────────────────────

(plum/register-installed-grammars!)
