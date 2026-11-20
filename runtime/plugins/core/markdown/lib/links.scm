;;; core:markdown — links.scm. See docs/links.md.

(require "destinations.scm")

(provide md-links/paste-link)

(define md-links/url-pattern (regex "^[a-z][a-z0-9+.-]*://[^\\s]+$"))

(define (md-links/matches? text pattern)
  (pair? (string-matches text pattern)))

;;; See docs/links.md's "Link paste".
(define (md-links/paste-link pane pasted previous)
  (and previous
       (not (string-contains? previous "\n"))
       (not (string-contains? previous "["))
       (not (string-contains? previous "]"))
       (not (ends-with? previous "\\"))
       (not (md-links/matches? previous md-links/url-pattern))
       (let ([destination (md-links/destination pane pasted)])
         (and destination (string-append "[" previous "](" destination ")")))))

(define (md-links/destination pane pasted)
  (let ([destination (md-destinations/encode pasted)])
    (and destination
         (or (md-links/matches? pasted md-links/url-pattern)
             (and (not (starts-with? pasted "#"))
                  (let ([path (md-destinations/resolve pane pasted)])
                    (and path (path-exists? path)))))
         destination)))
