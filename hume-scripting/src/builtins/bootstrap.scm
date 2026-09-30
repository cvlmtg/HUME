(require-builtin steel/meta as hm.)

(define (declare-plugin! name #:commands       [commands       '()]
                             #:typed-commands [typed-commands '()]
                             #:events         [events         '()]
                             #:languages      [languages      '()]
                             #:config         [config         (hash)])
  (if (and (null? commands) (null? typed-commands) (null? events) (null? languages))
      (let ((prog (%begin-manifest-declare! name config)))
        (when prog
          (with-handler
            (lambda (e) (%finish-manifest-declare! name e))
            (begin (hm.eval-string prog) (%finish-manifest-declare! name #f)))
          (hume/yield!)))
      (%declare-plugin! name commands typed-commands events languages config)))

(define (load-plugin! name #:config [config (hash)])
  (%load-plugin! name config)
  (%activate-plugin-inline! name))

(define (%activate-plugin-inline! id)
  (let ((prog (%begin-lazy-activation! id)))
    (when prog
      (with-handler
        (lambda (e) (%finish-lazy-activation! id e))
        (begin (hm.eval-string prog) (%finish-lazy-activation! id #f)))
      (hume/yield!))))

(define (define-command! name doc proc
                         #:repeatable    [repeatable    #f]
                         #:inline-output [inline-output #f])
  (%define-command! name doc proc repeatable inline-output))

(define (define-typed-command! name doc proc
                               #:inline-output [inline-output #f]
                               #:complete [complete #f])
  (%define-typed-command! name doc proc inline-output complete))

(define (%apply-command! proc name args)
  (let* ((depth (%arm-inline-output! name))
         (r (apply proc args)))
    (when depth (%restore-inline-output! depth))
    r))

(define (%dispatch-command! name args)
  (let ((proc (%lookup-plugin-proc name)))
    (if proc
        (%apply-command! proc name args)
        (let ((owner (%lazy-command-owner name)))
          (if owner
              (begin
                (%activate-plugin-inline! owner)
                (let ((proc2 (%lookup-plugin-proc name)))
                  (cond
                    (proc2 (%apply-command! proc2 name args))
                    ((member owner (loaded-plugins))
                     (error (string-append "'" name "': plugin '" owner
                                           "' loaded but did not define it")))
                    (else
                     (error (string-append "'" name "' unavailable: plugin '" owner
                                           "' failed to load"))))))
              (%call-native! name args))))))

(define (register-lsp-server! language #:command command
                                        #:args [args '()]
                                        #:root-markers [root-markers '()]
                                        #:init-options [init-options #f]
                                        #:settings [settings #f]
                                        #:env [env '()])
  (%register-lsp-server! language command args root-markers init-options settings env))

(define (lsp-request! pane method params callback #:allow-stale [allow-stale #f]
                                                   #:supersede [supersede #f]
                                                   #:require-focus [require-focus #f])
  (%lsp-request! pane method params callback allow-stale supersede require-focus))

(define (debounce ms proc)
  (let ((pending (box #f)))
    (lambda args
      (let ((prev (unbox pending)))
        (when prev (cancel-timer! prev)))
      (let ((my-id (box #f)))
        (set-box! my-id
          (after! ms (lambda ()
                      (when (equal? (unbox pending) (unbox my-id))
                        (set-box! pending #f))
                      (apply proc args))))
        (set-box! pending (unbox my-id))))))

(define (debounce-by ms proc #:key [key (lambda (first . _) first)])
  (let ((pending (box (hash))))
    (lambda args
      (let* ((k (apply key args))
             (table (unbox pending)))
        (when (hash-contains? table k)
          (cancel-timer! (hash-ref table k)))
        (let ((my-id (box #f)))
          (set-box! my-id
            (after! ms (lambda ()
                        (let ((table (unbox pending)))
                          (when (and (hash-contains? table k)
                                     (equal? (hash-ref table k) (unbox my-id)))
                            (set-box! pending (hash-remove table k))))
                        (apply proc args))))
          (set-box! pending (hash-insert (unbox pending) k (unbox my-id))))))))

(define (diagnostics-for-buffer pane #:severity [severity #f] #:range [range #f])
  (%diagnostics-for-buffer pane severity range))

(define (buffer-lines pane #:start [start #f] #:end [end #f])
  (%buffer-lines pane start end))

(define (apply-text-edits! pane edits #:expect-generation [gen #f])
  (%apply-text-edits! pane edits gen))

(define (apply-workspace-edit! pane wsedit #:expect-generation [gen #f])
  (let ((n (%apply-workspace-edit! pane wsedit gen)))
    (log! 'info (to-string n " buffers modified — :wa writes all"))
    n))

(define (prompt! pane label on-confirm #:prefill [prefill ""])
  (%prompt! pane label prefill on-confirm))

(define (register-completion-source! name proc #:target target
                                               #:match [match-kind 'fuzzy] #:priority [priority 0]
                                               #:resolve [resolve #f]
                                               #:token-chars [token-chars ""])
  (%register-completion-source! name proc target match-kind priority resolve token-chars))

(define (completion-emit! id items #:incomplete [incomplete #f])
  (%completion-emit! id items incomplete))

(define (spawn-async! cmd args callback #:cwd [cwd #f])
  (%spawn-async! cmd args cwd callback))

(define (run-capture! cmd args #:cwd [cwd #f])
  (%run-capture! cmd args cwd))

(define (run-inline-output! cmd args #:cwd [cwd #f])
  (let ([code (%run-inline-output! cmd args cwd)])
    (unless (= code 0)
      (error (string-append cmd ": failed (exit " (number->string code) ")")))))

(define (show-popup! pane text #:anchor [anchor 'cursor] #:kind [kind 'sticky] #:lang [lang #f])
  (%show-popup! pane text anchor kind lang))

(define (picker! pane items on-select #:prompt [prompt ""] #:pending [pending #f]
                                       #:query [query ""] #:truncate [truncate 'head]
                                       #:actions [actions '()])
  (%picker! pane items on-select prompt pending query truncate actions))

(define %picker-source-default-ok-exit-codes '(0))

(define (picker-source-spawn! token cmd args #:cwd [cwd #f] #:nul [nul #f]
                                              #:ok-exit-codes [ok-exit-codes %picker-source-default-ok-exit-codes])
  (%picker-source-spawn! token cmd args cwd nul ok-exit-codes))

(define (live-picker! pane on-select #:command command
                       #:prompt [prompt ""] #:query [query ""]
                       #:debounce-ms [debounce-ms 150]
                       #:cwd [cwd #f] #:nul [nul #f]
                       #:ok-exit-codes [ok-exit-codes %picker-source-default-ok-exit-codes]
                       #:truncate [truncate 'head]
                       #:actions [actions '()])
  (unless (%callable? command)
    (error "live-picker!: #:command must be a procedure of one argument (the query)"))
  (unless (and (integer? debounce-ms) (>= debounce-ms 0))
    (error "live-picker!: #:debounce-ms must be a non-negative integer"))
  (let* ([spawn-for (lambda (token q)
                       (let ([argv (command q)])
                         (if argv
                             (begin
                               (unless (and (list? argv) (not (null? argv)) (string? (car argv)))
                                 (error "live-picker!: #:command must return #f or a non-empty list of strings (argv)"))
                               (picker-source-spawn! token (car argv) (cdr argv)
                                                     #:cwd cwd #:nul nul #:ok-exit-codes ok-exit-codes))
                             (picker-replace! token '()))))]
         [respawn (debounce debounce-ms
                    (lambda (token q)
                      (with-handler
                        (lambda (e) (picker-replace! token '()) (raise-error e))
                        (spawn-for token q))))]
         [token (%live-picker! pane on-select prompt query
                  (lambda (token q)
                    (picker-source-stop! token)
                    (respawn token q))
                  truncate actions)])
    (unless (equal? query "")
      (spawn-for token query))
    token))


(define-syntax call!
  (syntax-rules ()
    ((_ name args ...)
     (%dispatch-command! name (list args ...)))))

(define %raw-displayln displayln)
(define %raw-display display)
(define %raw-print print)
(define %raw-println println)
(define %raw-newline newline)
(define %raw-write write)
(define %raw-write-string write-string)
(define %raw-write-char write-char)
(define %raw-simple-display simple-display)
(define %raw-simple-displayln simple-displayln)
(define %stdout-port (current-output-port))
(define (%port-safe? port)
  (if (eq? port %stdout-port)
      (%stdout-gate!)
      #t))
(define (%stdout-safe?) (%port-safe? (current-output-port)))
