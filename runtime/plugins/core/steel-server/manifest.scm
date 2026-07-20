; Default activation for `(declare-plugin "core:steel-server")` with no explicit
; #:commands/#:events/#:languages — see README.md "Usage".
(declare-plugin "core:steel-server"
  #:languages '("scheme")
  #:commands '("steel-server-install"))
