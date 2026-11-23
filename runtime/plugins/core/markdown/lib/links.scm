;;; core:markdown — links.scm. See docs/links.md.

(require "destinations.scm")
(require "inline.scm")
(require "queries.scm")
(require "render.scm")
(require "state.scm")

(provide md-links/paste-link md-links/follow! md-links/follow-waiting!)

(define md-links/url-pattern (regex "^[a-z][a-z0-9+.-]*://[^\\s]+$"))
(define md-links/scheme-pattern (regex "^[a-z][a-z0-9+.-]{1,31}$"))
(define md-links/opened-schemes '("http" "https" "mailto"))
(define md-links/image-pattern (regex "\\.(png|jpe?g|gif|webp|svg|bmp|ico|avif|tiff?)$"))

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
                  (let ([path (md-destinations/resolve pane (car (md-destinations/split pasted)))])
                    (and path (path-exists? path)))))
         destination)))

(define md-links/inline-query
  (string-append
    "(inline_link (link_destination) @destination) @inline\n"
    "(image (link_destination) @destination) @image\n"
    "(uri_autolink) @url\n"
    "(email_autolink) @email\n"
    "[(full_reference_link) (collapsed_reference_link) (shortcut_link)] @reference\n"))

(define md-links/heading-query
  (string-append
    "(atx_heading (inline) @text) @atx\n"
    "(setext_heading (paragraph (inline) @text)) @setext\n"))

(define md-links/closing-hashes (regex "([ \\t]+#+)?[ \\t]*$"))
(define md-links/slug-chars (regex "[\\p{L}\\p{N}_ -]+"))

;;; The follow waiting for its buffer's tree, or `#f`: a hash of `'pane`, showing that buffer at
;;; its start when its text was `'generation`, the `'fragment` it names, and the `'timer` that
;;; warns, `#f` once it has. See docs/links.md's "Following links".
(define md-links/*waiting* (box #f))
(define md-links/parse-wait-ms 1000)

(define (md-links/text pane capture)
  (buffer-text pane #:range (hash 'start-char (md-preview/capture-start capture)
                                  'end-char (md-preview/capture-end capture))))

;;; The inline conceals the preview draws on the lines `captures` lie on, in drawing order, or
;;; none without the `markdown.inline` grammar.
(define (md-links/conceals target captures)
  (if (language-has-grammar? "markdown.inline")
      (let* ([reader (md-preview/line-reader target)]
             [line-of (car reader)]
             [line-info (cdr reader)])
        (md-preview/drawn line-of line-info
          (md-preview/inline-elements
            (syntax-captures target "markdown.inline" md-preview/inline-query
                             #:ranges (map (lambda (c) (md-preview/line-span line-of c)) captures))
            line-of line-info)))
      '()))

;;; `text`, the buffer's text from offset `start`, as the preview shows it once `conceals` apply.
(define (md-links/shown-text text start conceals)
  (md-preview/shown-text (cons start (+ start (string-length text))) conceals
                         (md-preview/line-slicer (cons start text))))

;;; The match of the innermost link holding `head`, or `#f`. Its first entry
;;; is the link, named by its kind, and its last the text it targets.
(define (md-links/link-at pane head)
  (let ([line (char-offset->line pane head)])
    (foldl (lambda (found best)
             (let* ([match (if (equal? (car (car found)) "reference")
                               (list (md-links/bracketed pane (car found)))
                               found)]
                    [link (car match)])
               (if (and (<= (md-preview/capture-start link) head)
                        (< head (md-preview/capture-end link))
                        (or (not best) (md-preview/inside? link (car best))))
                   match
                   best)))
           #f
           (if (language-has-grammar? "markdown.inline")
               (syntax-captures pane "markdown.inline" md-links/inline-query
                                #:ranges (list (hash 'start-line line 'end-line (+ line 1))))
               '()))))

;;; The reference link `capture`, widened over the brackets around it when one more pair
;;; wraps it: the grammar reads a wiki link `[[b]]` as a shortcut link `[b]` between them.
(define (md-links/bracketed pane capture)
  (let ([start (md-preview/capture-start capture)]
        [end (md-preview/capture-end capture)])
    (if (and (> start 0)
             (let ([text (buffer-text pane #:range (hash 'start-char (- start 1) 'end-char (+ end 1)))])
               (and (starts-with? text "[[") (ends-with? text "]]"))))
        (list (car capture) (- start 1) (+ end 1))
        capture)))

;;; What the link `match` targets, as the preview shows it, an angle destination's `<` `>`
;;; dropped. See docs/links.md's "Following links".
(define (md-links/target pane match)
  (let* ([capture (last match)]
         [text (md-links/shown-text (md-links/text pane capture) (md-preview/capture-start capture)
                                    (md-links/conceals pane (list capture)))])
    (if (and (starts-with? text "<") (ends-with? text ">"))
        (substring text 1 (- (string-length text) 1))
        text)))

(define (md-links/ask-server! pane)
  (and (command-exists? "lsp/goto-definition")
       (begin (call! "lsp/goto-definition" pane) #t)))

;;; `target`'s scheme, lowercased, or `#f`. See docs/links.md's "Following links".
(define (md-links/scheme target)
  (let ([parts (split-once target ":")])
    (and (pair? parts)
         (md-links/matches? (car parts) md-links/scheme-pattern)
         (string-downcase (car parts)))))

;;; Opens `target`, a URL or an image's path, in the system's app, raising for any other. See
;;; docs/links.md's "Following links".
(define (md-links/open-externally! target)
  (let ([scheme (md-links/scheme target)])
    (cond
      [(and scheme (not (member scheme md-links/opened-schemes)))
       (error (string-append "markdown: g d opens only http, https and mailto links, not " scheme ":"))]
      [(not (or scheme (md-links/matches? target md-links/image-pattern)))
       (error (string-append "markdown: g d opens only an image's file in the system's app, not " target))]))
  (let* ([os (current-os!)]
         [command (cond [(equal? os "macos") (list "open" target)]
                        [(equal? os "windows") (list "rundll32" "url.dll,FileProtocolHandler" target)]
                        [else (list "xdg-open" target)])])
    (spawn-async!
      (car command)
      (cdr command)
      (lambda (result)
        (unless (equal? (hash-ref result 'exit) 0)
          (log! 'warning (string-append "markdown: could not open " target ": "
                                        (trim (hash-ref result 'stderr))))))))
  #t)

(define (md-links/slug text)
  (let ([lower (string-downcase text)])
    (string-replace
      (apply string-append (map (lambda (span) (substring lower (car span) (cdr span)))
                                (string-matches lower md-links/slug-chars)))
      " " "-")))

;;; A heading's shown text, an ATX heading's closing `#`s dropped, given its own `conceals`.
(define (md-links/heading-text target match conceals)
  (let* ([capture (cadr match)]
         [raw (md-links/text target capture)]
         [closing (and (equal? (car (car match)) "atx") (string-matches raw md-links/closing-hashes))]
         [kept (if (pair? closing) (substring raw 0 (car (car closing))) raw)])
    (trim (md-links/shown-text kept (md-preview/capture-start capture) conceals))))

;;; Each heading of `target` in order, as `(name . start)`. See docs/links.md's "Following links".
(define (md-links/named-headings target)
  (let* ([matches (syntax-captures target "markdown" md-links/heading-query)]
         [texts (map cadr matches)]
         [owned (md-preview/bucket (map md-preview/extent texts)
                                   (md-links/conceals target texts)
                                   (lambda (c) (hash-ref c 'start-char)))]
         [counts (box (hash))])
    (map (lambda (match conceals)
           (let* ([slug (md-links/slug (md-links/heading-text target match conceals))]
                  [n (or (hash-try-get (unbox counts) slug) 0)])
             (md-preview/box-insert! counts slug (+ n 1))
             (cons (if (= n 0) slug (string-append slug "-" (number->string n)))
                   (md-preview/capture-start (car match)))))
         matches
         owned)))

;;; See docs/links.md's "Following links".
(define (md-links/heading-at target fragment)
  (let ([wanted (string-downcase fragment)])
    (and (language-has-grammar? "markdown")
         (equal? (get-buffer-option target 'language) "markdown")
         (let ([named (assoc wanted (md-links/named-headings target))])
           (and named (cdr named))))))

(define (md-links/warn-missed fragment)
  (log! 'warning (string-append "markdown: no heading #" fragment)))

(define (md-links/goto! pane target offset)
  (goto-location! pane (hash 'target target 'char-offset offset)))

(define (md-links/stop-waiting!)
  (let ([waiting (unbox md-links/*waiting*)])
    (when waiting
      (let ([timer (hash-ref waiting 'timer)])
        (when timer (cancel-timer! timer)))
      (set-box! md-links/*waiting* #f))))

(define (md-links/warn-waiting!)
  (let ([waiting (unbox md-links/*waiting*)])
    (md-links/warn-missed (hash-ref waiting 'fragment))
    (set-box! md-links/*waiting* (hash-insert waiting 'timer #f))))

;;; Whether a heading of `target` may come with a parse still to land. See docs/links.md's
;;; "Following links".
(define (md-links/may-parse? target)
  (and (language-has-grammar? "markdown")
       (member (get-buffer-option target 'language) '("markdown" ""))))

;;; Show `target`'s buffer in `pane`'s pane, at the heading the percent-encoded `fragment`
;;; names, or at its start. See docs/links.md's "Following links".
(define (md-links/show! pane target encoded)
  (let* ([fragment (and encoded (not (equal? encoded "")) (md-destinations/decode encoded))]
         [offset (and fragment (md-links/heading-at target fragment))])
    (cond
      [offset (md-links/goto! pane target offset)]
      [(and fragment (md-links/may-parse? target))
       (let ([shown (md-links/goto! pane target 0)])
         (set-box! md-links/*waiting*
                   (hash 'pane shown 'fragment fragment 'generation (buffer-generation shown)
                         'timer (after! md-links/parse-wait-ms md-links/warn-waiting!))))]
      [else
       (when fragment (md-links/warn-missed fragment))
       (md-links/goto! pane target 0)])
    #t))

(define (md-links/follow-waiting! buffer)
  (let ([waiting (unbox md-links/*waiting*)])
    (when (and waiting (equal? (buffer-key (hash-ref waiting 'pane)) (buffer-key buffer)))
      (md-links/stop-waiting!)
      (let* ([pane (hash-ref waiting 'pane)]
             [fragment (hash-ref waiting 'fragment)]
             [offset (and (pane-live? pane)
                          (equal? (buffer-generation pane) (hash-ref waiting 'generation))
                          (= 0 (call! "stdlib/cursor-char-offset" (buffer-selections pane)))
                          (or (md-links/heading-at pane fragment) 'missed))])
        (cond
          [(equal? offset 'missed)
           (when (hash-ref waiting 'timer) (md-links/warn-missed fragment))]
          [offset (md-links/goto! pane pane offset)])))))

(define (md-links/follow-target! pane target image?)
  (let ([parts (md-destinations/split target)])
    (cond
      [(md-links/scheme target) (md-links/open-externally! target)]
      [(and (starts-with? target "#") (not image?)) (md-links/show! pane pane (cdr parts))]
      [else
       (let ([path (md-destinations/resolve pane (md-destinations/decode (car parts)))])
         (cond
           [(not path) (error (string-append "markdown: a buffer with no file can't follow " target))]
           [(not (path-exists? path)) (error (string-append "markdown: no file " path))]
           [(is-dir? path) (error (string-append "markdown: " path " is a folder"))]
           [(and image? (md-links/matches? path md-links/image-pattern)) (md-links/open-externally! path)]
           [else (md-links/show! pane (open-buffer! path) (cdr parts))]))])))

;;; See docs/links.md's "Following links".
(define (md-links/follow! pane)
  (md-links/stop-waiting!)
  (let ([match (md-links/link-at pane (call! "stdlib/cursor-char-offset" (buffer-selections pane)))])
    (and match
         (let ([kind (car (car match))])
           (cond
             [(equal? kind "reference") (md-links/ask-server! pane)]
             [(equal? kind "email")
              (md-links/open-externally! (string-append "mailto:" (md-links/target pane match)))]
             [else (md-links/follow-target! pane (md-links/target pane match) (equal? kind "image"))])))))
