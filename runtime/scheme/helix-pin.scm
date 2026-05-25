;;; runtime/scheme/helix-pin.scm — pinned helix-editor/helix commit SHA.
;;;
;;; PURE DATA. One literal string. Read via the R7RS idiom from any plugin:
;;;
;;;   (define helix-pin
;;;     (call-with-input-file
;;;       (path-join (runtime-dir) "scheme" "helix-pin.scm")
;;;       read))
;;;
;;; `helix-pin` is the single source of truth for both grammar revisions
;;; (in grammar-sources.scm) and Helix query URLs:
;;;
;;;   https://raw.githubusercontent.com/helix-editor/helix/<helix-pin>/runtime/queries/<lang>/highlights.scm
;;;
;;; To upgrade: change the SHA below, then run scripts/sync-grammars.py to
;;; regenerate languages.scm and grammar-sources.scm.

"8c41b1160792"
