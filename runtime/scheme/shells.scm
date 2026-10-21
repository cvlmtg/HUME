;;; runtime/scheme/shells.scm — HUME shell dialects for `|` completion.
;;; The dialect contract and each dialect's rules: README.md, this directory.

;; ── The word record ─────────────────────────────────────────────────────────

(struct shell-word (start end command? typed dir open prefix))

;; ── Registry ────────────────────────────────────────────────────────────────

(define *shell-dialects* '())

(define (register-shell-dialect! name
                                 #:programs programs
                                 #:program-extensions [program-extensions '(".exe")]
                                 #:parse parse
                                 #:write write)
  (unless (and (string? name) (list? programs) (list? program-extensions)
               (procedure? parse) (procedure? write))
    (error "register-shell-dialect!: expected a name, #:programs and #:program-extensions lists, and #:parse and #:write procedures"))
  (set! *shell-dialects*
        (cons (cons name (hash 'name name
                               'programs (map string-downcase programs)
                               'program-extensions program-extensions
                               'parse parse
                               'write write))
              (filter (lambda (entry) (not (equal? (car entry) name))) *shell-dialects*))))

(define (%shell-dialect-for program)
  (let* ((file (last (split-many (string-replace program "\\" "/") "/")))
         (name (string-downcase (if (ends-with? (string-downcase file) ".exe")
                                    (substring file 0 (- (string-length file) 4))
                                    file)))
         (entry (filter (lambda (entry) (member name (hash-ref (cdr entry) 'programs)))
                        *shell-dialects*)))
    (and (pair? entry) (cdr (car entry)))))

;; ── Shared scanning ─────────────────────────────────────────────────────────

(struct %shell-rules (escape escape-in quotes operators starters keywords separators tilde? read))

(struct %shell-unit (value n extendable?))

(define (%shell-blank? c) (memv c '(#\space #\tab #\newline)))

(define (%shell-any pred xs)
  (and (pair? xs) (or (pred (car xs)) (%shell-any pred (cdr xs)))))

(define (%shell-scan rules input cursor)
  (define escape (%shell-rules-escape rules))
  (define escape-in (%shell-rules-escape-in rules))
  (define quotes (%shell-rules-quotes rules))
  (define operators (%shell-rules-operators rules))
  (define starters (%shell-rules-starters rules))
  (define keywords (%shell-rules-keywords rules))
  (define separators (%shell-rules-separators rules))
  (define read (%shell-rules-read rules))
  (define start 0)
  (define open #f)
  (define last #f)
  (define before #f)
  (define typed-end 0)
  (define typed-open #f)
  (define typed-separator "")
  (define dir "")
  (define tail '())
  (define (starts-command?)
    (if (or (not before) (eq? before 'command) (memv before starters)) #t #f))
  (define (take! unit to)
    (when (<= to cursor)
      (let* ((value (%shell-unit-value unit))
             (n (string-length value)))
        (set! tail (cons value tail))
        (when (and (> n 0) (memv (string-ref value (- n 1)) separators))
          (set! dir (apply string-append (cons dir (reverse tail))))
          (set! tail '())
          (set! typed-end to)
          (set! typed-open open)
          (set! typed-separator
                (if (%shell-unit-extendable? unit) (substring value (- n 1) n) ""))))))
  (define (unit! unit i)
    (let ((to (+ i 1 (%shell-unit-n unit))))
      (take! unit to)
      to))
  (define (finish end)
    (shell-word start end (starts-command?)
                (string-append (substring input start typed-end) typed-separator) dir typed-open
                (apply string-append (reverse tail))))
  (define nothing (%shell-unit "" 0 #f))
  (let loop ((cs (string->list input)) (i 0))
    (if (null? cs)
        (finish i)
        (let ((c (car cs)) (rest (cdr cs)))
          (cond
            ((and escape (char=? c escape) (or (not open) (memv open escape-in)))
             (set! last c)
             (if (null? rest)
                 (loop rest (unit! nothing i))
                 (let ((r (or (read open c rest) (%shell-unit (string (car rest)) 1 #f))))
                   (loop (list-tail rest (%shell-unit-n r)) (unit! r i)))))
            ((and open (char=? c open))
             (set! last c)
             (let ((r (read open c rest)))
               (if r
                   (loop (list-tail rest (%shell-unit-n r)) (unit! r i))
                   (begin (set! open #f)
                          (loop rest (unit! nothing i))))))
            ((and (not open) (memv c quotes))
             (set! last c)
             (set! open c)
             (when (and (= i typed-end) (<= (+ i 1) cursor))
               (set! typed-end (+ i 1))
               (set! typed-open c)
               (set! typed-separator ""))
             (loop rest (unit! nothing i)))
            ((and (not open) (or (%shell-blank? c) (memv c operators)))
             (if (>= i cursor)
                 (finish i)
                 (begin
                   (set! last (cond ((memv c operators) c)
                                    ((and (member (substring input start i) keywords)
                                          (starts-command?))
                                     'command)
                                    (else last)))
                   (set! before last)
                   (set! start (+ i 1))
                   (set! typed-end (+ i 1))
                   (set! typed-open #f)
                   (set! typed-separator "")
                   (set! dir "")
                   (set! tail '())
                   (loop rest (+ i 1)))))
            ((and (not open) (%shell-rules-tilde? rules) (= i start) (char=? c #\~)
                  (pair? rest) (memv (car rest) separators) (home-dir))
             (set! last c)
             (loop rest (unit! (%shell-unit (home-dir) 0 #f) i)))
            (else
             (set! last c)
             (let ((r (or (read open c rest) (%shell-unit (string c) 0 #f))))
               (loop (list-tail rest (%shell-unit-n r)) (unit! r i)))))))))

(define (%shell-map-chars name escape-char)
  (if (not (%shell-any escape-char (string->list name)))
      name
      (apply string-append (map (lambda (c) (or (escape-char c) (string c))) (string->list name)))))

(define (%shell-var-char? c first?)
  (let ((n (char->integer c)))
    (or (and (>= n 65) (<= n 90)) (and (>= n 97) (<= n 122)) (= n 95)
        (and (not first?) (>= n 48) (<= n 57)))))

(define (%shell-var-name cs)
  (let loop ((cs cs) (name '()) (first? #t))
    (if (and (pair? cs) (%shell-var-char? (car cs) first?))
        (loop (cdr cs) (cons (car cs) name) #f)
        (reverse name))))

(define (%shell-env name literal n extendable?)
  (let ((value (maybe-get-env-var (list->string name))))
    (%shell-unit (if (Ok? value) (Ok->value value) literal) n extendable?)))

(define (%shell-dollar cs braces?)
  (if (and braces? (pair? cs) (char=? (car cs) #\{))
      (let* ((name (%shell-var-name (cdr cs)))
             (after (list-tail (cdr cs) (length name))))
        (if (and (pair? name) (pair? after) (char=? (car after) #\}))
            (%shell-env name (string-append "${" (list->string name) "}") (+ (length name) 2) #f)
            (%shell-unit "${" 1 #f)))
      (let ((name (%shell-var-name cs)))
        (and (pair? name)
             (%shell-env name (string-append "$" (list->string name)) (length name) #t)))))

(define (%shell-control-unit c)
  (case c [(#\n) (%shell-unit "\n" 1 #f)] [(#\t) (%shell-unit "\t" 1 #f)] [(#\r) (%shell-unit "\r" 1 #f)] [else #f]))

(define (%shell-close open close?)
  (if (and open close?) (string open) ""))

;; ── POSIX sh ────────────────────────────────────────────────────────────────

(define (%posix-read open c rest)
  (cond
    ((char=? c #\\)
     (and (eqv? open #\") (not (memv (car rest) '(#\$ #\` #\" #\\ #\newline)))
          (%shell-unit "\\" 0 #f)))
    ((and (char=? c #\$) (not (eqv? open #\'))) (%shell-dollar rest #t))
    (else #f)))

(define %posix-rules
  (%shell-rules #\\ '(#\") '(#\' #\") '(#\| #\; #\& #\( #\) #\< #\>) '(#\| #\; #\& #\() '()
               '(#\/) #t %posix-read))

(define (shell/posix-parse input cursor)
  (%shell-scan %posix-rules input cursor))

(define %posix-special
  (string->list " \t\\'\"`$&;|()<>*?[]{}!#~"))

(define (shell/posix-write word name close?)
  (let ((open (shell-word-open word)))
    (string-append
     (shell-word-typed word)
     (%shell-map-chars name
       (cond
         ((eqv? open #\') (lambda (c) (and (char=? c #\') "'\\''")))
         ((eqv? open #\") (lambda (c) (and (memv c '(#\$ #\` #\" #\\)) (string #\\ c))))
         (else (lambda (c) (cond ((char=? c #\newline) "'\n'")
                                 ((memv c %posix-special) (string #\\ c))
                                 (else #f))))))
     (%shell-close open close?))))

(register-shell-dialect! "posix"
  #:programs '("sh" "bash" "zsh" "dash" "ksh")
  #:parse shell/posix-parse
  #:write shell/posix-write)

;; ── fish ────────────────────────────────────────────────────────────────────

(define (%fish-read open c rest)
  (cond
    ((char=? c #\\)
     (cond
       ((not open)
        (%shell-control-unit (car rest)))
       ((memv (car rest) (if (eqv? open #\') '(#\' #\\) '(#\" #\$ #\\))) #f)
       (else (%shell-unit "\\" 0 #f))))
    ((and (char=? c #\$) (not (eqv? open #\'))) (%shell-dollar rest #f))
    (else #f)))

(define %fish-rules
  (%shell-rules #\\ '(#\' #\") '(#\' #\") '(#\| #\; #\& #\( #\) #\< #\>) '(#\| #\; #\& #\()
               '("and" "or" "not") '(#\/) #t %fish-read))

(define (shell/fish-parse input cursor)
  (%shell-scan %fish-rules input cursor))

(define %fish-special
  (string->list "\\'\"$&;|()<>*?~#{}[] "))

(define (shell/fish-write word name close?)
  (let ((open (shell-word-open word)))
    (string-append
     (shell-word-typed word)
     (%shell-map-chars name
       (cond
         ((eqv? open #\') (lambda (c) (and (memv c '(#\' #\\)) (string #\\ c))))
         ((eqv? open #\") (lambda (c) (and (memv c '(#\" #\$ #\\)) (string #\\ c))))
         (else (lambda (c) (cond ((char=? c #\newline) "\\n")
                                 ((char=? c #\tab) "\\t")
                                 ((memv c %fish-special) (string #\\ c))
                                 (else #f))))))
     (%shell-close open close?))))

(register-shell-dialect! "fish"
  #:programs '("fish")
  #:parse shell/fish-parse
  #:write shell/fish-write)

;; ── Windows shells ──────────────────────────────────────────────────────────

(define %shell-pathext
  (let ((pathext (maybe-get-env-var "PATHEXT")))
    (filter (lambda (ext) (not (equal? ext "")))
            (split-many (if (Ok? pathext) (Ok->value pathext) ".COM;.EXE;.BAT;.CMD") ";"))))

(define (%shell-dir-separator name separator)
  (if (ends-with? name "/")
      (string-append (substring name 0 (- (string-length name) 1)) separator)
      name))

;; ── PowerShell ──────────────────────────────────────────────────────────────

(define (%pwsh-dollar cs)
  (define (after-prefix prefix cs)
    (let loop ((p (string->list prefix)) (cs cs))
      (cond ((null? p) cs)
            ((and (pair? cs) (char=? (char-downcase (car cs)) (car p))) (loop (cdr p) (cdr cs)))
            (else #f))))
  (cond
    ((after-prefix "{env:" cs)
     => (lambda (after)
          (let* ((name (%shell-var-name after))
                 (rest (list-tail after (length name))))
            (and (pair? name) (pair? rest) (char=? (car rest) #\})
                 (%shell-env name (string-append "${env:" (list->string name) "}")
                             (+ 6 (length name)) #f)))))
    ((after-prefix "env:" cs)
     => (lambda (after)
          (let ((name (%shell-var-name after)))
            (and (pair? name)
                 (%shell-env name (string-append "$env:" (list->string name)) (+ 4 (length name))
                             #t)))))
    ((after-prefix "home" cs)
     => (lambda (after)
          (and (not (and (pair? after) (%shell-var-char? (car after) #f)))
               (home-dir)
               (%shell-unit (home-dir) 4 #t))))
    (else #f)))

(define (%pwsh-read open c rest)
  (cond
    ((char=? c #\`)
     (and (eqv? open #\") (%shell-control-unit (car rest))))
    ((and open (char=? c open) (pair? rest) (char=? (car rest) open)) (%shell-unit (string open) 1 #f))
    ((and (char=? c #\$) (not (eqv? open #\'))) (%pwsh-dollar rest))
    (else #f)))

(define %pwsh-rules
  (%shell-rules #\` '(#\") '(#\' #\") '(#\| #\; #\& #\( #\) #\{ #\} #\< #\> #\,)
               '(#\| #\; #\& #\( #\{) '() '(#\\ #\/) #t %pwsh-read))

(define (shell/pwsh-parse input cursor)
  (%shell-scan %pwsh-rules input cursor))

(define %pwsh-special
  (string->list " \t'\"`$&;|(){}<>,@#"))

(define %pwsh-separator (if (equal? (current-os!) "windows") "\\" "/"))

(define (shell/pwsh-write word name close?)
  (let ((open (shell-word-open word))
        (typed (shell-word-typed word))
        (name (%shell-dir-separator name %pwsh-separator)))
    (cond
      ((eqv? open #\')
       (string-append typed (string-replace name "'" "''") (%shell-close open close?)))
      ((eqv? open #\")
       (string-append typed
                      (%shell-map-chars name (lambda (c) (and (memv c '(#\` #\" #\$)) (string #\` c))))
                      (%shell-close open close?)))
      (else
       (let ((path (string-append (shell-word-dir word) name)))
         (if (%shell-any (lambda (c) (memv c %pwsh-special)) (string->list path))
             (string-append "'" (string-replace path "'" "''") (%shell-close #\' close?))
             (string-append typed name)))))))

(register-shell-dialect! "pwsh"
  #:programs '("pwsh" "powershell")
  #:program-extensions (append %shell-pathext '(".ps1"))
  #:parse shell/pwsh-parse
  #:write shell/pwsh-write)

;; ── cmd ─────────────────────────────────────────────────────────────────────

(define (%cmd-read open c rest)
  (and (char=? c #\%)
       (let* ((name (%shell-var-name rest))
              (after (list-tail rest (length name)))
              (value (and (pair? name) (pair? after) (char=? (car after) #\%)
                          (maybe-get-env-var (list->string name)))))
         (and value (Ok? value) (%shell-unit (Ok->value value) (+ (length name) 1) #f)))))

(define %cmd-rules
  (%shell-rules #\^ '() '(#\") '(#\& #\| #\< #\> #\( #\)) '(#\& #\| #\() '() '(#\\ #\/) #f %cmd-read))

(define (shell/cmd-parse input cursor)
  (%shell-scan %cmd-rules input cursor))

(define %cmd-special
  (string->list " \t&|<>^(),;="))

(define (shell/cmd-write word name close?)
  (let ((open (shell-word-open word))
        (name (%shell-dir-separator name "\\")))
    (if open
        (string-append (shell-word-typed word) name (%shell-close open close?))
        (let ((path (string-append (shell-word-dir word) name)))
          (if (%shell-any (lambda (c) (memv c %cmd-special)) (string->list path))
              (string-append "\"" path (%shell-close #\" close?))
              (string-append (shell-word-typed word) name))))))

(register-shell-dialect! "cmd"
  #:programs '("cmd")
  #:program-extensions %shell-pathext
  #:parse shell/cmd-parse
  #:write shell/cmd-write)

;; ── The `|` completion source ───────────────────────────────────────────────

(define (%shell-parse dialect input cursor)
  (let ((word ((hash-ref dialect 'parse) input cursor)))
    (unless (and (shell-word? word)
                 (integer? (shell-word-start word)) (integer? (shell-word-end word))
                 (<= 0 (shell-word-start word) cursor (shell-word-end word) (string-length input))
                 (string? (shell-word-typed word)) (string? (shell-word-dir word))
                 (string? (shell-word-prefix word)))
      (error (string-append "shell dialect " (hash-ref dialect 'name)
                            ": #:parse must return a shell-word whose span holds the cursor, with string typed, dir and prefix")))
    word))

(define (%shell-complete id input cursor)
  (let* ((program (car (get-option "shell")))
         (dialect (%shell-dialect-for program)))
    (if (not dialect)
        (begin
          (log! 'warn (string-append "| completion: no shell dialect for " program
                                     "; register one with register-shell-dialect!"))
          (completion-emit! id '()))
        (let* ((word (%shell-parse dialect input cursor))
               (write-word (hash-ref dialect 'write))
               (prefix (shell-word-prefix word))
               (names (if (and (shell-word-command? word) (equal? (shell-word-dir word) ""))
                          (path-programs prefix (hash-ref dialect 'program-extensions))
                          (path-entries (shell-word-dir word) prefix)))
               (item (lambda (name)
                       (if (ends-with? name "/")
                           (hash "label" name "insertText" (write-word word name #t) "kind" 19)
                           (hash "label" name "insertText" (write-word word name #t)))))
               (common (string-common-prefix names)))
          (completion-emit! id (map item names)
                            #:span (cons (shell-word-start word) (shell-word-end word))
                            #:prefix (and (> (length names) 1)
                                          (> (string-length common) (string-length prefix))
                                          (write-word word common #f)))))))

(register-completion-source! "shell" %shell-complete #:target 'minibuf #:match 'delegated)
