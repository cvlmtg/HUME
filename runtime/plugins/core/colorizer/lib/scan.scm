;;; core:colorizer — scan.scm. See README.md.

(require "names.scm")
(require "convert.scm")

(provide colorizer/scan colorizer/group)

(struct colorizer/group (lines offset))

(struct colorizer/text-line (text len mark pieces) #:mutable)

(define colorizer/*unclosed* (box #f))

(define-syntax colorizer/letter-code?
  (syntax-rules ()
    [(_ e) (let ([k e]) (if (< k 97) (and (<= 65 k) (<= k 90)) (<= k 122)))]))

(define-syntax colorizer/digit-code?
  (syntax-rules ()
    [(_ e) (let ([k e]) (and (<= 48 k) (<= k 57)))]))

(define-syntax colorizer/word-code?
  (syntax-rules ()
    [(_ e) (let ([k e])
             (or (colorizer/letter-code? k) (colorizer/digit-code? k) (= k 95) (= k 45)))]))

(define-syntax colorizer/sign-code?
  (syntax-rules ()
    [(_ e) (let ([k e]) (or (= k 43) (= k 45)))]))

(define-syntax colorizer/space-code?
  (syntax-rules ()
    [(_ e) (let ([k e]) (or (= k 32) (= k 9) (= k 13) (= k 12)))]))

(define colorizer/functions
  (hash "rgb" 'rgb "rgba" 'rgb "hsl" 'hsl "hsla" 'hsl "oklch" 'oklch))

(define colorizer/function-initials
  (let ([initials (map (lambda (name) (string-ref name 0)) (hash-keys->list colorizer/functions))])
    (apply hash
           (apply append
                  (map (lambda (ch) (list (char->integer ch) #t (char->integer (char-upcase ch)) #t))
                       initials)))))

(define colorizer/degrees-per-unit
  (hash "deg" 1.0 "grad" 0.9 "rad" (/ 45.0 (atan 1.0)) "turn" 360.0))

(define (colorizer/first chars n)
  (if (= n 0) '() (cons (car chars) (colorizer/first (cdr chars) (- n 1)))))

(define (colorizer/word chars n)
  (string-downcase (list->string (colorizer/first chars n))))

;; ── Hex literals ──────────────────────────────────────────────────────────────

(define (colorizer/hex-color digits n)
  (cond
    [(= n 6) (list->string (reverse digits))]
    [(= n 8) (list->string (reverse (cddr digits)))]
    [else
     (let* ([rgb (if (= n 4) (cdr digits) digits)]
            [b (car rgb)]
            [g (cadr rgb)]
            [r (caddr rgb)])
       (list->string (list r r g g b b)))]))

(define (colorizer/hex-literal chars names?)
  (let loop ([cs chars] [n 0] [digits '()] [letter? #f])
    (let ([k (if (and (pair? cs) (< n 9)) (char->integer (car cs)) 0)])
      (cond
        [(colorizer/digit-code? k) (loop (cdr cs) (+ n 1) (cons (car cs) digits) letter?)]
        [(and (<= 97 k) (<= k 102)) (loop (cdr cs) (+ n 1) (cons (car cs) digits) #t)]
        [(and (<= 65 k) (<= k 70)) (loop (cdr cs) (+ n 1) (cons (char-downcase (car cs)) digits) #t)]
        [(and (or (null? cs) (not (colorizer/word-code? (char->integer (car cs)))))
              (or (= n 6) (= n 8) (and (or (= n 3) (= n 4)) (or names? letter?))))
         (list n digits cs)]
        [else #f]))))

;; ── Call cursor ───────────────────────────────────────────────────────────────

(struct colorizer/cursor (chars pos) #:mutable)

(define (colorizer/peek c)
  (let ([cs (colorizer/cursor-chars c)])
    (if (null? cs) -1 (char->integer (car cs)))))

(define (colorizer/next-is? c k)
  (let ([cs (colorizer/cursor-chars c)])
    (and (pair? cs) (pair? (cdr cs)) (= (char->integer (cadr cs)) k))))

(define (colorizer/advance! c)
  (set-colorizer/cursor-chars! c (cdr (colorizer/cursor-chars c)))
  (set-colorizer/cursor-pos! c (+ (colorizer/cursor-pos c) 1)))

(define (colorizer/take! c acc)
  (let ([ch (car (colorizer/cursor-chars c))])
    (colorizer/advance! c)
    (cons ch acc)))

;; ── Call tokens ───────────────────────────────────────────────────────────────

(define (colorizer/comment! c)
  (let ([k (colorizer/peek c)])
    (cond
      [(= k -1) #f]
      [(and (= k 42) (colorizer/next-is? c 47))
       (begin (colorizer/advance! c) (colorizer/advance! c) #t)]
      [else (begin (colorizer/advance! c) (colorizer/comment! c))])))

(define (colorizer/skip! c)
  (let ([k (colorizer/peek c)])
    (cond
      [(colorizer/space-code? k) (begin (colorizer/advance! c) (colorizer/skip! c))]
      [(and (= k 47) (colorizer/next-is? c 42)) (colorizer/skip-comment! c)]
      [else #t])))

(define (colorizer/skip-comment! c)
  (let ([start (colorizer/cursor-pos c)]
        [unclosed (unbox colorizer/*unclosed*)])
    (and (not (and unclosed (>= start unclosed)))
         (begin
           (colorizer/advance! c)
           (colorizer/advance! c)
           (if (colorizer/comment! c)
               (colorizer/skip! c)
               (begin (set-box! colorizer/*unclosed* start) #f))))))

(define (colorizer/letters! c acc)
  (if (colorizer/letter-code? (colorizer/peek c))
      (colorizer/letters! c (colorizer/take! c acc))
      (string-downcase (list->string (reverse acc)))))

(define (colorizer/digits! c acc)
  (if (colorizer/digit-code? (colorizer/peek c))
      (colorizer/digits! c (colorizer/take! c acc))
      acc))

(define (colorizer/dot-digit? c)
  (let ([cs (colorizer/cursor-chars c)])
    (and (pair? cs)
         (= (char->integer (car cs)) 46)
         (pair? (cdr cs))
         (colorizer/digit-code? (char->integer (cadr cs))))))

(define (colorizer/exponent? c)
  (let ([cs (colorizer/cursor-chars c)])
    (and (pair? cs)
         (let ([k (char->integer (car cs))])
           (or (= k 101) (= k 69)))
         (pair? (cdr cs))
         (let ([k (char->integer (cadr cs))])
           (or (colorizer/digit-code? k)
               (and (colorizer/sign-code? k)
                    (pair? (cddr cs))
                    (colorizer/digit-code? (char->integer (caddr cs)))))))))

(define (colorizer/number! c)
  (let ([signed (if (colorizer/sign-code? (colorizer/peek c)) (colorizer/take! c '()) '())])
    (and (or (colorizer/digit-code? (colorizer/peek c)) (colorizer/dot-digit? c))
         (let* ([whole (colorizer/digits! c signed)]
                [fraction (if (colorizer/dot-digit? c)
                              (colorizer/digits! c (colorizer/take! c whole))
                              whole)]
                [all (if (colorizer/exponent? c)
                         (let ([e (colorizer/take! c fraction)])
                           (colorizer/digits! c (if (colorizer/sign-code? (colorizer/peek c))
                                                    (colorizer/take! c e)
                                                    e)))
                         fraction)]
                [n (string->number (list->string (reverse all)))])
           (and n
                (let ([x (exact->inexact n)])
                  (and (finite? x) x)))))))

(define colorizer/none (cons 'none 0.0))

(define (colorizer/value! c)
  (let ([k (colorizer/peek c)])
    (cond
      [(or (colorizer/digit-code? k) (colorizer/sign-code? k) (= k 46))
       (let ([n (colorizer/number! c)])
         (and n
              (let ([k (colorizer/peek c)])
                (cond
                  [(= k 37) (begin (colorizer/advance! c) (cons 'percent n))]
                  [(colorizer/letter-code? k)
                   (let ([scale (hash-try-get colorizer/degrees-per-unit (colorizer/letters! c '()))])
                     (and scale (cons 'angle (* n scale))))]
                  [else (cons 'number n)]))))]
      [(colorizer/letter-code? k)
       (and (equal? (colorizer/letters! c '()) "none") colorizer/none)]
      [else #f])))

(define (colorizer/arg! c)
  (let ([v (colorizer/value! c)])
    (and v (colorizer/skip! c) v)))

(define (colorizer/comma-arg! c)
  (and (= (colorizer/peek c) 44)
       (begin (colorizer/advance! c) (colorizer/skip! c))
       (colorizer/arg! c)))

;; ── Call values ───────────────────────────────────────────────────────────────

(define (colorizer/scaled v number-unit percent-unit angle?)
  (let ([kind (car v)])
    (cond
      [(eq? kind 'number) (and number-unit (/ (cdr v) number-unit))]
      [(eq? kind 'percent) (and percent-unit (/ (cdr v) percent-unit))]
      [(eq? kind 'angle) (and angle? (cdr v))]
      [else (cdr v)])))

(define (colorizer/none? v)
  (eq? (car v) 'none))

(define (colorizer/hue-arg v)
  (colorizer/scaled v 1.0 #f #t))

(define (colorizer/color function legacy? v1 v2 v3)
  (and (not (and legacy? (or (colorizer/none? v1) (colorizer/none? v2) (colorizer/none? v3))))
       (cond
         [(eq? function 'rgb)
          (and (or (not legacy?) (and (eq? (car v1) (car v2)) (eq? (car v2) (car v3))))
               (let ([r (colorizer/scaled v1 255.0 100.0 #f)]
                     [g (colorizer/scaled v2 255.0 100.0 #f)]
                     [b (colorizer/scaled v3 255.0 100.0 #f)])
                 (and r g b (colorizer/rgb->hex r g b))))]
         [(eq? function 'hsl)
          (let ([h (colorizer/hue-arg v1)]
                [s (colorizer/scaled v2 (if legacy? #f 100.0) 100.0 #f)]
                [l (colorizer/scaled v3 (if legacy? #f 100.0) 100.0 #f)])
            (and h s l (colorizer/hsl->hex h s l)))]
         [else
          (and (not legacy?)
               (let ([lightness (colorizer/scaled v1 1.0 100.0 #f)]
                     [chroma (colorizer/scaled v2 1.0 250.0 #f)]
                     [hue (colorizer/hue-arg v3)])
                 (and lightness chroma hue (colorizer/oklch->hex lightness chroma hue))))])))

;; ── Calls ─────────────────────────────────────────────────────────────────────

(define (colorizer/alpha? v legacy?)
  (and v
       (not (eq? (car v) 'angle))
       (not (and legacy? (colorizer/none? v)))))

(define (colorizer/alpha! c legacy?)
  (let ([k (colorizer/peek c)])
    (cond
      [(and legacy? (= k 44)) (colorizer/alpha? (colorizer/comma-arg! c) #t)]
      [(and (not legacy?) (= k 47))
       (begin (colorizer/advance! c)
              (and (colorizer/skip! c) (colorizer/alpha? (colorizer/arg! c) #f)))]
      [else #t])))

(define (colorizer/close! c)
  (and (= (colorizer/peek c) 41)
       (begin (colorizer/advance! c) #t)))

(define (colorizer/call function c)
  (and (colorizer/skip! c)
       (let ([v1 (colorizer/arg! c)])
         (and v1
              (let* ([legacy? (= (colorizer/peek c) 44)]
                     [next-arg! (if legacy? colorizer/comma-arg! colorizer/arg!)]
                     [v2 (next-arg! c)]
                     [v3 (and v2 (next-arg! c))])
                (and v3
                     (colorizer/alpha! c legacy?)
                     (colorizer/close! c)
                     (colorizer/color function legacy? v1 v2 v3)))))))

;; ── Lines ─────────────────────────────────────────────────────────────────────

(define (colorizer/scan-call function pos after after-pos names? acc)
  (let* ([c (colorizer/cursor after after-pos)]
         [hex (colorizer/call function c)])
    (if hex
        (let ([end (colorizer/cursor-pos c)])
          (colorizer/scan-chars (colorizer/cursor-chars c) end 41 names?
                                (cons (list pos end hex) acc)))
        (colorizer/scan-chars after after-pos 40 names? acc))))

(define (colorizer/scan-word cs pos names? acc)
  (let count ([ws cs] [n 0])
    (if (and (pair? ws) (colorizer/letter-code? (char->integer (car ws))))
        (count (cdr ws) (+ n 1))
        (let* ([end (+ pos n)]
               [k (if (pair? ws) (char->integer (car ws)) -1)]
               [call? (and (= k 40) (<= 3 n) (<= n 5)
                           (hash-contains? colorizer/function-initials (char->integer (car cs))))]
               [name? (and names?
                           (<= colorizer/shortest-name n)
                           (<= n colorizer/longest-name)
                           (not (colorizer/word-code? k)))]
               [word (and (or call? name?) (colorizer/word cs n))]
               [function (and call? (hash-try-get colorizer/functions word))])
          (if function
              (colorizer/scan-call function pos (cdr ws) (+ end 1) names? acc)
              (let ([hex (and name? (hash-try-get colorizer/names word))])
                (colorizer/scan-chars ws end 97 names?
                                      (if hex (cons (list pos end hex) acc) acc))))))))

(define (colorizer/scan-chars cs pos prev names? acc)
  (if (null? cs)
      acc
      (let ([k (char->integer (car cs))])
        (cond
          [(and (= k 35) (not (= prev 38)) (not (colorizer/word-code? prev)))
           (let ([hit (colorizer/hex-literal (cdr cs) names?)])
             (if hit
                 (let ([end (+ pos 1 (car hit))])
                   (colorizer/scan-chars (caddr hit) end 48 names?
                                         (cons (list pos end (colorizer/hex-color (cadr hit) (car hit)))
                                               acc)))
                 (colorizer/scan-chars (cdr cs) (+ pos 1) k names? acc)))]
          [(and (colorizer/letter-code? k) (not (colorizer/word-code? prev)))
           (colorizer/scan-word cs pos names? acc)]
          [else (colorizer/scan-chars (cdr cs) (+ pos 1) k names? acc)]))))

(define (colorizer/line-pieces line offset names? known)
  (let ([mark (colorizer/text-line-mark line)])
    (cond
      [(eq? mark 'known) (colorizer/text-line-pieces line)]
      [(eq? mark 'skip) '()]
      [else
       (set-box! colorizer/*unclosed* #f)
       (let ([pieces (colorizer/scan-chars (string->list (colorizer/text-line-text line))
                                           offset 0 names? '())])
         (colorizer/box-insert! known offset (cons (colorizer/text-line-len line) pieces))
         pieces)])))

(define (colorizer/scan-lines lines offset names? known acc)
  (if (null? lines)
      acc
      (let ([line (car lines)])
        (colorizer/scan-lines (cdr lines)
                              (+ offset (colorizer/text-line-len line) 1)
                              names?
                              known
                              (append (colorizer/line-pieces line offset names? known) acc)))))

;; ── Render budget ─────────────────────────────────────────────────────────────

(define (colorizer/text-lines lines offset table)
  (if (null? lines)
      '()
      (let* ([text (car lines)]
             [entry (hash-try-get table offset)]
             [len (if entry (car entry) (string-length text))])
        (cons (colorizer/text-line text len (and entry 'known) (and entry (cdr entry)))
              (colorizer/text-lines (cdr lines) (+ offset len 1) table)))))

(define (colorizer/claim! lines left)
  (when (pair? lines)
    (let ([line (car lines)])
      (if (colorizer/text-line-mark line)
          (colorizer/claim! (cdr lines) left)
          (let ([len (colorizer/text-line-len line)])
            (if (<= len left)
                (begin (set-colorizer/text-line-mark! line 'walk)
                       (colorizer/claim! (cdr lines) (- left len)))
                (begin (set-colorizer/text-line-mark! line 'skip)
                       (colorizer/claim! (cdr lines) left))))))))

(define (colorizer/scan groups budget known names?)
  (let* ([table (unbox known)]
         [lines (map (lambda (group)
                       (colorizer/text-lines (colorizer/group-lines group)
                                             (colorizer/group-offset group)
                                             table))
                     groups)])
    (colorizer/claim! (apply append lines) budget)
    (let loop ([groups groups] [lines lines] [acc '()])
      (if (null? groups)
          (reverse acc)
          (loop (cdr groups)
                (cdr lines)
                (colorizer/scan-lines (car lines)
                                      (colorizer/group-offset (car groups))
                                      names?
                                      known
                                      acc))))))
