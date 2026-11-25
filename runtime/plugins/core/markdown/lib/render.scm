;;; core:markdown — render.scm. See docs/architecture.md.

(require "state.scm")
(require "queries.scm")
(require "blocks.scm")
(require "inline.scm")
(require "table.scm")

(provide md-preview/render-buffer! md-preview/clear! md-preview/elements md-preview/line-of md-preview/line-reader
         md-preview/lines-line-of md-preview/lines-info md-preview/captures md-preview/drawn)

(define md-preview/source "markdown-preview")

(define (md-preview/clear! pane)
  (set-conceals! pane md-preview/source '())
  (set-line-backgrounds! pane md-preview/source '())
  (set-extra-highlights! pane md-preview/source '()))

(define (md-preview/render-ranges pane line-count)
  (let ([shown (buffer-viewport-ranges pane)])
    (append (map (lambda (range)
                   (hash 'start-line (max 0 (- (hash-ref range 'start-line) 1))
                         'end-line (min line-count (+ (hash-ref range 'end-line) 1))))
                 (hash-ref shown 'ranges))
            (hash-ref shown 'headers))))

;;; A procedure giving the line of `pane`'s text a char offset lies on.
(define (md-preview/line-of pane) (lambda (char) (char-offset->line pane char)))

;;; A reader of the lines of a text: `(line-of char)` is the line a char lies on, and `(info line)` the line's
;;; `(start-char . text)`.
(struct md-preview/lines (line-of info))

(define (md-preview/line-reader pane)
  (md-preview/lines
    (md-preview/line-of pane)
    (md-preview/memo
      (lambda (line)
        (cons (line->char-offset pane line)
              (car (buffer-lines pane #:range (hash 'start-line line 'end-line (+ line 1)))))))))

;;; The matches of `query` over `pane`'s `language` layers on `ranges`, none without that grammar.
(define (md-preview/captures pane language query ranges)
  (if (language-has-grammar? language)
      (syntax-captures pane language query #:ranges ranges)
      '()))

(define (md-preview/fence-contents matches)
  (map car (filter (lambda (m) (equal? (car (car m)) "fence-content")) matches)))

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

(define (md-preview/outside-fences matches zones contents)
  (let loop ([ms matches] [zones zones] [kept '()])
    (if (null? ms)
        (reverse kept)
        (let ([first (car (car ms))])
          (cond
            [(and (pair? zones)
                  (< (md-preview/capture-end (car zones)) (md-preview/capture-start first)))
             (loop ms (cdr zones) kept)]
            [(and (pair? zones)
                  (md-preview/inside? first (car zones))
                  (not (md-preview/root-continuation? first contents)))
             (loop (cdr ms) zones kept)]
            [else (loop (cdr ms) zones (cons (car ms) kept))])))))

;;; Each of `conceals`, which come sorted by start, cut at the end of the line it starts on.
(define (md-preview/clamped line-of line-info conceals)
  (let loop ([cs conceals] [line-end -1] [out '()])
    (if (null? cs)
        (reverse out)
        (let* ([c (car cs)]
               [start (hash-ref c 'start-char)]
               [line-end (if (<= start line-end)
                             line-end
                             (md-preview/line-end (line-info (line-of start))))])
          (loop (cdr cs)
                line-end
                (cons (if (<= (hash-ref c 'end-char) line-end) c (hash-insert c 'end-char line-end))
                      out))))))

;;; `conceals` as the preview draws them: in `md-preview/conceal-before?` order, clamped.
(define (md-preview/drawn line-of line-info conceals)
  (md-preview/clamped line-of line-info (sort conceals md-preview/conceal-before?)))

(define (md-preview/elements blocks inline-captures ranges line-of line-info line-count rule-width)
  (let* ([contents (md-preview/fence-contents blocks)]
         [zones (md-preview/fence-zones contents)]
         [groups (md-preview/group (md-preview/outside-fences blocks zones contents))]
         [elements (md-preview/block-elements groups ranges line-of line-info rule-width)]
         [table-runs (md-preview/runs-of groups "table")]
         [inline-ranges (append ranges
                                (map (lambda (run) (md-preview/line-span line-of (car run))) table-runs))]
         [inline (if (language-has-grammar? "markdown.inline")
                     (md-preview/inline-elements
                       (md-preview/outside-fences (inline-captures inline-ranges) zones contents)
                       line-of line-info)
                     '())]
         [tables (md-preview/table-elements table-runs inline ranges line-of line-info line-count)])
    (hash 'conceals (md-preview/drawn line-of line-info
                                      (append (hash-ref elements 'conceals)
                                              (hash-ref tables 'conceals)
                                              (hash-ref tables 'inline)))
          'bands (hash-ref elements 'bands)
          'highlights (hash-ref elements 'highlights))))

(define (md-preview/draw! pane ranges line-count rule-width)
  (let* ([lines (md-preview/line-reader pane)]
         [blocks (syntax-captures pane "markdown" md-preview/block-query #:ranges ranges)]
         [found (md-preview/elements
                  blocks
                  (lambda (inline-ranges)
                    (syntax-captures pane "markdown.inline" md-preview/inline-query
                                     #:ranges inline-ranges))
                  ranges (md-preview/lines-line-of lines) (md-preview/lines-info lines) line-count rule-width)])
    (set-conceals! pane md-preview/source (hash-ref found 'conceals) #:overlaps 'drop)
    (set-line-backgrounds! pane md-preview/source (hash-ref found 'bands))
    (set-extra-highlights! pane md-preview/source (hash-ref found 'highlights))))

(define (md-preview/render-buffer! pane rule-width)
  (let ([entry (md-preview/entry pane)])
    (when (and entry
               (hash-ref entry 'enabled?)
               (equal? (hash-ref entry 'generation) (buffer-generation pane)))
      (let* ([line-count (buffer-line-count pane)]
             [ranges (md-preview/render-ranges pane line-count)])
        (unless (null? ranges)
          (md-preview/draw! pane ranges line-count rule-width))))))
