; Default activation for `(declare-plugin "core:git-diff")` with no
; explicit #:commands/#:events/#:languages — see README.md "Usage" and
; "Customizing activation".
;
; #:events '(on-buffer-open): signs default on (README's Config table), so
; the plugin must wake on the first buffer opened, not wait for a command
; typed by hand. #:commands covers the two toggles, for a user who declares
; an explicit activation list instead of taking this manifest's defaults.
(declare-plugin "core:git-diff"
  #:events '(on-buffer-open)
  #:commands '("toggle-git-signs" "toggle-inline-diff"))
