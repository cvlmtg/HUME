;;; core:colorizer/manifest.scm — see README.md.
(declare-plugin! "core:colorizer"
  #:events '(on-buffer-open)
  #:commands '("colorizer/toggle")
  #:typed-commands '("toggle-colorizer"))
