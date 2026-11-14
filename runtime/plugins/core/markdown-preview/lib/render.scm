;;; core:markdown-preview — render.scm. See README.md.

(require "state.scm")
(require "queries.scm")
(require "blocks.scm")
(require "inline.scm")
(require "table.scm")

(provide md-preview/render-buffer! md-preview/clear!)

(define md-preview/source "markdown-preview")

(define (md-preview/clear! pane)
  (set-conceals! pane md-preview/source '())
  (set-line-backgrounds! pane md-preview/source '())
  (set-extra-highlights! pane md-preview/source '()))

(define (md-preview/render-ranges panes)
  (map (lambda (pane)
         (let ([shown (viewport-range pane)])
           (hash-insert shown 'start-line (max 0 (- (hash-ref shown 'start-line) 1)))))
       panes))

(define (md-preview/line-reader pane)
  (let ([memo (box (hash))])
    (cons
      (lambda (char) (char-offset->line pane char))
      (lambda (line)
        (or (hash-try-get (unbox memo) line)
            (let ([info (cons (line->char-offset pane line)
                              (car (buffer-lines pane #:range (hash 'start-line line 'end-line (+ line 1)))))])
              (set-box! memo (hash-insert (unbox memo) line info))
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

(define (md-preview/disjoint conceals)
  (let loop ([cs (sort conceals md-preview/conceal-before?)]
             [hidden-start -1] [hidden-end -1] [inserted-at -1] [kept '()])
    (if (null? cs)
        (reverse kept)
        (let* ([c (car cs)]
               [start (hash-ref c 'start-char)]
               [end (hash-ref c 'end-char)])
          (cond
            [(< start end)
             (if (< start hidden-end)
                 (loop (cdr cs) hidden-start hidden-end inserted-at kept)
                 (loop (cdr cs) start end inserted-at (cons c kept)))]
            [(or (= start inserted-at) (and (< hidden-start start) (< start hidden-end)))
             (loop (cdr cs) hidden-start hidden-end inserted-at kept)]
            [else (loop (cdr cs) hidden-start hidden-end start (cons c kept))])))))

(define (md-preview/draw! pane ranges rule-width)
  (let* ([lines (md-preview/line-reader pane)]
         [line-of (car lines)]
         [line-info (cdr lines)]
         [blocks (syntax-captures pane "markdown" md-preview/block-query #:ranges ranges)]
         [contents (md-preview/fence-contents blocks)]
         [groups (md-preview/group (md-preview/outside-fences blocks contents))]
         [elements (md-preview/block-elements groups ranges line-of line-info rule-width)]
         [inline-ranges (append ranges
                                (md-preview/table-spans (md-preview/captured groups "table") line-of))]
         [inline (if (language-has-grammar? "markdown.inline")
                     (md-preview/inline-elements
                       (md-preview/outside-fences
                         (syntax-captures pane "markdown.inline" md-preview/inline-query
                                          #:ranges inline-ranges)
                         contents)
                       line-of line-info)
                     '())]
         [tables (md-preview/table-elements groups inline ranges line-of line-info
                                            (buffer-line-count pane))])
    (set-conceals! pane md-preview/source
                   (md-preview/disjoint
                     (map (lambda (c) (md-preview/clamped line-of line-info c))
                          (append (hash-ref elements 'conceals)
                                  (hash-ref tables 'conceals)
                                  (hash-ref tables 'inline)))))
    (set-line-backgrounds! pane md-preview/source (hash-ref elements 'bands))
    (set-extra-highlights! pane md-preview/source (hash-ref elements 'highlights))))

(define (md-preview/render-buffer! pane rule-width)
  (let ([entry (md-preview/entry pane)])
    (when (and entry
               (hash-ref entry 'enabled?)
               (equal? (hash-ref entry 'generation) (buffer-generation pane)))
      (let ([panes (buffer-panes pane)])
        (unless (null? panes)
          (md-preview/draw! pane (md-preview/render-ranges panes) rule-width))))))
