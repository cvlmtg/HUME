;;; core:colorizer — convert.scm. See README.md.

(provide colorizer/rgb->hex colorizer/hsl->hex colorizer/oklch->hex colorizer/srgb->linear
         colorizer/min2 colorizer/max2 colorizer/box-insert!)

(define colorizer/jnd 0.02)

(define colorizer/epsilon 0.0001)

(define colorizer/radians-per-degree (/ (atan 1.0) 45.0))

(define colorizer/inverse-gamma (/ 1.0 2.4))

(define colorizer/one-third (/ 1.0 3.0))

(define (colorizer/min2 a b) (if (< a b) a b))

(define (colorizer/max2 a b) (if (> a b) a b))

(define (colorizer/clamp v low high)
  (if (>= v low) (if (> v high) high v) low))

(define (colorizer/hue degrees)
  (- degrees (* 360.0 (floor (/ degrees 360.0)))))

;; ── Bytes ─────────────────────────────────────────────────────────────────────

(define (colorizer/byte-hex v)
  (let* ([byte (inexact->exact (floor (+ (* (colorizer/clamp v 0.0 1.0) 255.0) 0.5)))]
         [digits (number->string byte 16)])
    (if (< byte 16) (string-append "0" digits) digits)))

(define (colorizer/rgb->hex r g b)
  (string-append (colorizer/byte-hex r) (colorizer/byte-hex g) (colorizer/byte-hex b)))

;; ── hsl ───────────────────────────────────────────────────────────────────────

(define (colorizer/hsl-channel n h l a)
  (let* ([k (+ n (/ h 30.0))]
         [k (if (>= k 12.0) (- k 12.0) k)])
    (- l (* a (colorizer/max2 -1.0 (colorizer/min2 (- k 3.0) (colorizer/min2 (- 9.0 k) 1.0)))))))

(define (colorizer/hsl->hex hue s l)
  (let* ([h (colorizer/hue hue)]
         [s (colorizer/clamp s 0.0 1.0)]
         [l (colorizer/clamp l 0.0 1.0)]
         [a (* s (colorizer/min2 l (- 1.0 l)))])
    (colorizer/rgb->hex (colorizer/hsl-channel 0.0 h l a)
                        (colorizer/hsl-channel 8.0 h l a)
                        (colorizer/hsl-channel 4.0 h l a))))

;; ── sRGB and OKLab ────────────────────────────────────────────────────────────

(define (colorizer/linear->srgb c)
  (if (<= c 0.0031308)
      (* 12.92 c)
      (- (* 1.055 (expt c colorizer/inverse-gamma)) 0.055)))

(define (colorizer/srgb->linear c)
  (if (<= c 0.04045)
      (/ c 12.92)
      (expt (/ (+ c 0.055) 1.055) 2.4)))

(define-syntax colorizer/let-srgb
  (syntax-rules ()
    [(_ (red green blue) (lightness a b) body ...)
     (let* ([l* (+ lightness (* 0.3963377774 a) (* 0.2158037573 b))]
            [m* (- lightness (* 0.1055613458 a) (* 0.0638541728 b))]
            [s* (- lightness (* 0.0894841775 a) (* 1.2914855480 b))]
            [l (* l* l* l*)]
            [m (* m* m* m*)]
            [s (* s* s* s*)]
            [red (colorizer/linear->srgb (+ (* 4.0767416621 l) (* -3.3077115913 m) (* 0.2309699292 s)))]
            [green (colorizer/linear->srgb (+ (* -1.2684380046 l) (* 2.6097574011 m) (* -0.3413193965 s)))]
            [blue (colorizer/linear->srgb (+ (* -0.0041960863 l) (* -0.7034186147 m) (* 1.7076147010 s)))])
       body ...)]))

(define (colorizer/oklab->srgb lightness a b)
  (colorizer/let-srgb (red green blue) (lightness a b) (list red green blue)))

;; ── Gamut mapping ─────────────────────────────────────────────────────────────

(define (colorizer/unit? c)
  (and (>= c 0.0) (<= c 1.0)))

(define (colorizer/clip rgb)
  (list (colorizer/clamp (car rgb) 0.0 1.0)
        (colorizer/clamp (cadr rgb) 0.0 1.0)
        (colorizer/clamp (caddr rgb) 0.0 1.0)))

(define (colorizer/clip-error lightness a b)
  (colorizer/let-srgb (red green blue) (lightness a b)
    (if (and (colorizer/unit? red) (colorizer/unit? green) (colorizer/unit? blue))
        -1.0
        (let* ([r (colorizer/srgb->linear (colorizer/clamp red 0.0 1.0))]
               [g (colorizer/srgb->linear (colorizer/clamp green 0.0 1.0))]
               [b* (colorizer/srgb->linear (colorizer/clamp blue 0.0 1.0))]
               [l (expt (+ (* 0.4122214708 r) (* 0.5363325363 g) (* 0.0514459929 b*)) colorizer/one-third)]
               [m (expt (+ (* 0.2119034982 r) (* 0.6806995451 g) (* 0.1073969566 b*)) colorizer/one-third)]
               [s (expt (+ (* 0.0883024619 r) (* 0.2817188376 g) (* 0.6299787005 b*)) colorizer/one-third)]
               [dl (- (+ (* 0.2104542553 l) (* 0.7936177850 m) (* -0.0040720468 s)) lightness)]
               [da (- (+ (* 1.9779984951 l) (* -2.4285922050 m) (* 0.4505937099 s)) a)]
               [db (- (+ (* 0.0259040371 l) (* 0.7827717662 m) (* -0.8086757660 s)) b)])
          (sqrt (+ (* dl dl) (* da da) (* db db)))))))

(define (colorizer/gamut-map lightness chroma cos-h sin-h)
  (let ([at (lambda (c) (colorizer/oklab->srgb lightness (* c cos-h) (* c sin-h)))]
        [e (colorizer/clip-error lightness (* chroma cos-h) (* chroma sin-h))])
    (cond
      [(< e 0.0) (at chroma)]
      [(< e colorizer/jnd) (colorizer/clip (at chroma))]
      [else
       (let search ([low 0.0] [high chroma] [low-in-gamut? #t] [clipped-at chroma])
         (if (<= (- high low) colorizer/epsilon)
             (colorizer/clip (at clipped-at))
             (let* ([middle (/ (+ low high) 2.0)]
                    [e (colorizer/clip-error lightness (* middle cos-h) (* middle sin-h))])
               (cond
                 [(< e 0.0) (search middle high low-in-gamut? (if low-in-gamut? clipped-at middle))]
                 [(< e colorizer/jnd)
                  (if (< (- colorizer/jnd e) colorizer/epsilon)
                      (colorizer/clip (at middle))
                      (search middle high #f middle))]
                 [else (search low middle low-in-gamut? middle)]))))])))

(define (colorizer/oklch-convert lightness chroma hue)
  (let ([lightness (colorizer/clamp lightness 0.0 1.0)])
    (cond
      [(>= lightness 1.0) "ffffff"]
      [(<= lightness 0.0) "000000"]
      [else
       (let* ([radians (* (colorizer/hue hue) colorizer/radians-per-degree)]
              [rgb (colorizer/gamut-map lightness (colorizer/max2 chroma 0.0)
                                        (cos radians) (sin radians))])
         (colorizer/rgb->hex (car rgb) (cadr rgb) (caddr rgb)))])))

(define (colorizer/box-insert! table-box key value)
  (let ([table (unbox table-box)])
    (set-box! table-box #f)
    (set-box! table-box (hash-insert table key value))))

(define colorizer/*oklch* (box (hash)))

(define (colorizer/oklch->hex lightness chroma hue)
  (let ([key (list lightness chroma hue)])
    (or (hash-try-get (unbox colorizer/*oklch*) key)
        (let ([hex (colorizer/oklch-convert lightness chroma hue)])
          (colorizer/box-insert! colorizer/*oklch* key hex)
          hex))))
