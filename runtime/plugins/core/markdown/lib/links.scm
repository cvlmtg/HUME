;;; core:markdown — links.scm. See docs/links.md.

(provide md-links/paste-link)

(define md-links/url-pattern (regex "^[a-z][a-z0-9+.-]*://[^\\s]+$"))

(define md-links/breaks-link-pattern (regex "[\\[\\]]|\\\\$"))

(define (md-links/matches? text pattern)
  (pair? (string-matches text pattern)))

;;; See docs/links.md's "Link paste".
(define (md-links/paste-link pasted previous)
  (and previous
       (md-links/matches? pasted md-links/url-pattern)
       (not (string-contains? previous "\n"))
       (not (md-links/matches? previous md-links/breaks-link-pattern))
       (not (md-links/matches? previous md-links/url-pattern))
       (string-append "[" previous "](" pasted ")")))
