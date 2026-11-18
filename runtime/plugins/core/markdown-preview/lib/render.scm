;;; core:markdown-preview — render.scm. See README.md.

(require "state.scm")
(require "queries.scm")
(require "blocks.scm")
(require "inline.scm")
(require "table.scm")

(provide md-preview/render-buffer! md-preview/clear! md-preview/elements)

(define md-preview/source "markdown-preview")

(define (md-preview/clear! pane)
  (set-conceals! pane md-preview/source '())
  (set-line-backgrounds! pane md-preview/source '())
  (set-extra-highlights! pane md-preview/source '()))

(define (md-preview/render-ranges pane)
  (let ([shown (buffer-viewport-ranges pane)])
    (append (map (lambda (range)
                   (hash-insert range 'start-line (max 0 (- (hash-ref range 'start-line) 1))))
                 (hash-ref shown 'ranges))
            (hash-ref shown 'headers))))

(define (md-preview/line-reader pane)
  (let ([memo (box (hash))])
    (cons
      (lambda (char) (char-offset->line pane char))
      (lambda (line)
        (or (hash-try-get (unbox memo) line)
            (let ([info (cons (line->char-offset pane line)
                              (car (buffer-lines pane #:range (hash 'start-line line 'end-line (+ line 1)))))])
              (md-preview/box-insert! memo line info)
              info))))))

(define (md-preview/fence-contents captures)
  (filter (lambda (c) (equal? (car c) "fence-content")) captures))

(define (md-preview/fence-zones contents)
  (let loop ([cs contents] [reach -1] [zones '()])
    (cond
      [(null? cs) (reverse zones)]
      [(>= (md-preview/capture-start (car cs)) reach)
       (loop (cdr cs) (md-preview/capture-end (car cs)) (cons (car cs) zones))]
      [else (loop (cdr cs) reach zones)])))

(define (md-preview/root-continuation? capture contents)
  (and (equal? (car capture) "fence-continuation")
       (= 1 (length (filter (lambda (content) (md-preview/inside? capture content)) contents)))))

(define (md-preview/outside-fences captures contents)
  (let loop ([cs captures] [zones (md-preview/fence-zones contents)] [kept '()])
    (cond
      [(null? cs) (reverse kept)]
      [(and (pair? zones)
            (< (md-preview/capture-end (car zones)) (md-preview/capture-start (car cs))))
       (loop cs (cdr zones) kept)]
      [(and (pair? zones)
            (md-preview/inside? (car cs) (car zones))
            (not (md-preview/root-continuation? (car cs) contents)))
       (loop (cdr cs) zones kept)]
      [else (loop (cdr cs) zones (cons (car cs) kept))])))

(define (md-preview/clamped line-of line-info conceal)
  (let* ([info (line-info (line-of (hash-ref conceal 'start-char)))]
         [line-end (+ (car info) (string-length (cdr info)))])
    (if (<= (hash-ref conceal 'end-char) line-end)
        conceal
        (hash-insert conceal 'end-char line-end))))

(define (md-preview/elements blocks inline-captures ranges line-of line-info line-count rule-width)
  (let* ([contents (md-preview/fence-contents blocks)]
         [groups (md-preview/group (md-preview/outside-fences blocks contents))]
         [elements (md-preview/block-elements groups ranges line-of line-info rule-width)]
         [inline-ranges (append ranges
                                (md-preview/table-spans (md-preview/captured groups "table") line-of))]
         [inline (md-preview/inline-elements
                   (md-preview/outside-fences (inline-captures inline-ranges) contents)
                   line-of line-info)]
         [tables (md-preview/table-elements groups inline ranges line-of line-info line-count)])
    (hash 'conceals (sort (map (lambda (c) (md-preview/clamped line-of line-info c))
                               (append (hash-ref elements 'conceals)
                                       (hash-ref tables 'conceals)
                                       (hash-ref tables 'inline)))
                          md-preview/conceal-before?)
          'bands (hash-ref elements 'bands)
          'highlights (hash-ref elements 'highlights))))

(define (md-preview/draw! pane ranges rule-width)
  (let* ([lines (md-preview/line-reader pane)]
         [blocks (syntax-captures pane "markdown" md-preview/block-query #:ranges ranges)]
         [found (md-preview/elements
                  blocks
                  (lambda (inline-ranges)
                    (if (language-has-grammar? "markdown.inline")
                        (syntax-captures pane "markdown.inline" md-preview/inline-query
                                         #:ranges inline-ranges)
                        '()))
                  ranges (car lines) (cdr lines) (buffer-line-count pane) rule-width)])
    (set-conceals! pane md-preview/source (hash-ref found 'conceals) #:overlaps 'drop)
    (set-line-backgrounds! pane md-preview/source (hash-ref found 'bands))
    (set-extra-highlights! pane md-preview/source (hash-ref found 'highlights))))

(define (md-preview/render-buffer! pane rule-width)
  (let ([entry (md-preview/entry pane)])
    (when (and entry
               (hash-ref entry 'enabled?)
               (equal? (hash-ref entry 'generation) (buffer-generation pane)))
      (let ([ranges (md-preview/render-ranges pane)])
        (unless (null? ranges)
          (md-preview/draw! pane ranges rule-width))))))
