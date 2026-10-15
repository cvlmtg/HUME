;;; core:git-diff/manifest.scm — see README.md.
(declare-plugin! "core:git-diff"
  #:events '(on-buffer-open)
  #:commands '("git-diff/render-diff" "git-diff/release-diff"
              "git-diff/toggle-git-signs" "git-diff/toggle-inline-diff"
              "git-diff/show-inline-diff" "git-diff/inline-diff?")
  #:typed-commands '("toggle-git-signs" "toggle-inline-diff"))
