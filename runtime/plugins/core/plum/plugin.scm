;;; core:plum — HUME's plugin and grammar manager
;;;
;;; Manages third-party Steel plugins and tree-sitter grammars.
;;;
;;; Plugin commands:   :plum-install  :plum-cleanup  :plum-update  :plum-list
;;; Grammar commands:  :plum-install-grammar  :plum-update-grammar
;;;                    :plum-ensure-grammars  :plum-list-grammars
;;;                    :plum-cleanup-grammars
;;;
;;; Usage in init.scm (add before other third-party plugin declarations):
;;;   (load-plugin "core:plum")
;;;
;;; To auto-install missing grammars at startup, add after the load:
;;;   (call! "plum-ensure-grammars")

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
