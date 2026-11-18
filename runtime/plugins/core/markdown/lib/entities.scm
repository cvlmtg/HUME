;;; core:markdown — entities.scm. See README.md.

(provide md-preview/entity-text)

(define md-preview/named-entities
  (hash "amp" "&" "lt" "<" "gt" ">" "quot" "\"" "apos" "'"
        "nbsp" (string (integer->char #xA0)) "copy" "©" "reg" "®" "trade" "™"
        "mdash" "—" "ndash" "–" "hellip" "…"))

(define (md-preview/code-point body)
  (if (or (starts-with? body "#x") (starts-with? body "#X"))
      (string->number (substring body 2) 16)
      (string->number (substring body 1) 10)))

(define (md-preview/shown-char n)
  (and (integer? n)
       (< #x1F n)
       (not (and (<= #x7F n) (<= n #x9F)))
       (not (and (<= #xD800 n) (<= n #xDFFF)))
       (<= n #x10FFFF)
       (string (integer->char n))))

(define (md-preview/entity-text source)
  (let ([body (substring source 1 (- (string-length source) 1))])
    (if (starts-with? body "#")
        (md-preview/shown-char (md-preview/code-point body))
        (hash-try-get md-preview/named-entities body))))
