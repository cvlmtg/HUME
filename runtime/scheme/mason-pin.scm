;;; runtime/scheme/mason-pin.scm — pinned mason-org/mason-registry release tag.
;;;
;;; PURE DATA. One literal string. Read via the R7RS idiom from any plugin:
;;;
;;;   (define mason-pin
;;;     (call-with-input-file
;;;       (path-join (runtime-dir) "scheme" "mason-pin.scm")
;;;       read))
;;;
;;; `mason-pin` is the single source of truth for install sources (download
;;; URLs, asset sha256s) in lsp-sources.scm:
;;;
;;;   https://github.com/mason-org/mason-registry/releases/download/<mason-pin>/registry.json.zip
;;;
;;; To upgrade: change the tag below, then run scripts/sync-lsp-sources.py to
;;; regenerate lsp-sources.scm. If the preceding helix-pin.scm bump renamed or
;;; dropped any LSP servers, run scripts/sync-grammars.py first — see
;;; scripts/README.md for the run order.

"2026-07-20-precious-hemp"
