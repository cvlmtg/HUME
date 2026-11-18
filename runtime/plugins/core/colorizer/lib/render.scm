;;; core:colorizer — render.scm. See README.md.

(require "scan.scm")
(require "convert.scm")

(provide colorizer/options colorizer/track-buffer! colorizer/remove-buffer!
         colorizer/toggle-enabled! colorizer/render-buffer! colorizer/clear!)

(struct colorizer/options (enabled? mode named-languages max-render-chars))

(define colorizer/source "colorizer")

(define colorizer/*buffers* (box (hash)))

(define colorizer/*sent* (box (hash)))

(define colorizer/*full* (box #f))

(define colorizer/full-warning
  (string-append "colorizer: the session holds as many scope names as it allows, "
                 "so a color not painted before stays unpainted"))

;; ── Buffer state ──────────────────────────────────────────────────────────────

(struct colorizer/state (enabled? generation names? ranges lines))

(define (colorizer/entry pane)
  (hash-try-get (unbox colorizer/*buffers*) (buffer-key pane)))

(define (colorizer/put-entry! pane entry)
  (set-box! colorizer/*buffers*
            (hash-insert (unbox colorizer/*buffers*) (buffer-key pane) entry)))

(define (colorizer/track-buffer! pane options)
  (unless (colorizer/entry pane)
    (colorizer/put-entry! pane (colorizer/state (colorizer/options-enabled? options) #f #f #f
                                                (box (hash))))))

(define (colorizer/remove-buffer! pane)
  (set-box! colorizer/*buffers*
            (hash-remove (unbox colorizer/*buffers*) (buffer-key pane))))

(define (colorizer/toggle-enabled! pane)
  (let* ([state (colorizer/entry pane)]
         [on? (not (colorizer/state-enabled? state))])
    (colorizer/put-entry! pane (colorizer/state on?
                                                (colorizer/state-generation state)
                                                (colorizer/state-names? state)
                                                #f
                                                (colorizer/state-lines state)))
    on?))

;; ── Styles ────────────────────────────────────────────────────────────────────

(define (colorizer/linear-channel hex i)
  (colorizer/srgb->linear (/ (string->number (substring hex i (+ i 2)) 16) 255.0)))

(define (colorizer/luminance hex)
  (+ (* 0.2126 (colorizer/linear-channel hex 0))
     (* 0.7152 (colorizer/linear-channel hex 2))
     (* 0.0722 (colorizer/linear-channel hex 4))))

(define (colorizer/style scope hex background?)
  (let ([color (string-append "#" hex)])
    (if background?
        (hash 'scope scope
              'bg color
              'fg (if (> (colorizer/luminance hex) 0.179) "#000000" "#ffffff"))
        (hash 'scope scope 'fg color))))

;; ── Rendering ─────────────────────────────────────────────────────────────────

(define (colorizer/render-pieces pane ranges names? budget lines)
  (colorizer/scan (map (lambda (range)
                         (colorizer/group (buffer-lines pane #:range range)
                                          (line->char-offset pane (hash-ref range 'start-line))))
                       ranges)
                  budget
                  lines
                  names?))

(define (colorizer/highlight! pane spans)
  (with-handler
    (lambda (err)
      (if (string-contains? (error-text err) "too many distinct scope names")
          #f
          (raise-error err)))
    (begin (set-extra-highlights! pane colorizer/source spans) #t)))

(define (colorizer/paint! pane options ranges names? lines)
  (let* ([background? (equal? (colorizer/options-mode options) 'background)]
         [prefix (if background? "colorizer.bg." "colorizer.fg.")]
         [full? (unbox colorizer/*full*)])
    (let loop ([ps (colorizer/render-pieces pane ranges names?
                                            (colorizer/options-max-render-chars options) lines)]
               [spans '()]
               [fresh (hash)])
      (if (null? ps)
          (let ([hexes (hash-keys->list fresh)]
                [spans (reverse spans)])
            (unless (null? hexes)
              (set-scope-styles!
                (map (lambda (hex) (colorizer/style (hash-ref fresh hex) hex background?)) hexes)))
            (cond
              [full? (set-extra-highlights! pane colorizer/source spans)]
              [(colorizer/highlight! pane spans)
               (for-each (lambda (hex) (colorizer/box-insert! colorizer/*sent* hex (hash-ref fresh hex)))
                         hexes)]
              [else
               (set-box! colorizer/*full* #t)
               (log! 'warning colorizer/full-warning)
               (colorizer/paint! pane options ranges names? lines)]))
          (let* ([piece (car ps)]
                 [hex (caddr piece)]
                 [sent (hash-try-get (unbox colorizer/*sent*) hex)]
                 [known (or sent (hash-try-get fresh hex))]
                 [scope (or known (string-append prefix hex))])
            (if (and full? (not sent))
                (loop (cdr ps) spans fresh)
                (loop (cdr ps)
                      (cons (hash 'start-char (car piece) 'end-char (cadr piece) 'scope scope) spans)
                      (if known fresh (hash-insert fresh hex scope)))))))))

(define (colorizer/shown-ranges pane)
  (let ([shown (buffer-viewport-ranges pane)])
    (sort (append (hash-ref shown 'ranges) (hash-ref shown 'headers))
          (lambda (a b) (< (hash-ref a 'start-line) (hash-ref b 'start-line))))))

(define (colorizer/render-buffer! pane options)
  (when (buffer-live? pane)
    (colorizer/track-buffer! pane options)
    (let ([state (colorizer/entry pane)])
      (when (colorizer/state-enabled? state)
        (let* ([ranges (colorizer/shown-ranges pane)]
               [generation (buffer-generation pane)]
               [names? (and (member (get-buffer-option pane "language")
                                    (colorizer/options-named-languages options))
                            #t)]
               [same-text? (and (equal? generation (colorizer/state-generation state))
                                (eq? names? (colorizer/state-names? state)))])
          (unless (or (null? ranges)
                      (and same-text? (equal? ranges (colorizer/state-ranges state))))
            (let ([lines (if same-text? (colorizer/state-lines state) (box (hash)))])
              (colorizer/paint! pane options ranges names? lines)
              (colorizer/put-entry! pane (colorizer/state #t generation names? ranges lines)))))))))

(define (colorizer/clear! pane)
  (set-extra-highlights! pane colorizer/source '()))
