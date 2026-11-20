;;; core:markdown — links.scm. See docs/links.md.

(require "destinations.scm")

(provide md-links/paste-link)

(define md-links/url-pattern (regex "^[a-z][a-z0-9+.-]*://[^\\s]+$"))

(define md-links/breaks-link-pattern (regex "[\\[\\]]|\\\\$"))

(define (md-links/matches? text pattern)
  (pair? (string-matches text pattern)))

;;; See docs/links.md's "Link paste".
(define (md-links/paste-link pane pasted previous)
  (and previous
       (not (string-contains? previous "\n"))
       (not (md-links/matches? previous md-links/breaks-link-pattern))
       (not (md-links/matches? previous md-links/url-pattern))
       (let ([destination (md-links/destination pane pasted)])
         (and destination (string-append "[" previous "](" destination ")")))))

(define (md-links/destination pane pasted)
  (if (md-links/matches? pasted md-links/url-pattern)
      (md-destinations/encode pasted)
      (let ([path (and (not (starts-with? pasted "#"))
                       (not (string-contains? pasted "\n"))
                       (md-destinations/resolve pane pasted))])
        (and path (path-exists? path) (md-destinations/encode pasted)))))
