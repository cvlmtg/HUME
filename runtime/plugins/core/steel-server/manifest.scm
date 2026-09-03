; Default activation for `(declare-plugin "core:steel-server")` with no explicit
; #:commands/#:typed-commands/#:events/#:languages — see README.md "Usage".
(declare-plugin "core:steel-server"
  #:languages '("scheme")
  #:typed-commands '("steel-server-install"))
