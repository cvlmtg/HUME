(require-builtin steel/meta as hm.)

(define (declare-plugin name #:commands       [commands       '()]
                             #:typed-commands [typed-commands '()]
                             #:events         [events         '()]
                             #:languages      [languages      '()]
                             #:config         [config         (hash)])
  (if (and (null? commands) (null? typed-commands) (null? events) (null? languages))
      (let ((prog (%begin-manifest-declare! name config)))
        (when prog
          (with-handler
            (lambda (e) (%finish-manifest-declare! name #f) (raise-error e))
            (begin (hm.eval-string prog) (%finish-manifest-declare! name #t)))))
      (%declare-plugin! name commands typed-commands events languages config)))

(define (load-plugin name #:config [config (hash)])
  (%load-plugin! name config)
  (%activate-plugin-inline name))

(define (%activate-plugin-inline id)
  (let ((prog (%begin-lazy-activation id)))
    (when prog
      (with-handler
        (lambda (e) (%finish-lazy-activation id #f) (raise-error e))
        (begin (hm.eval-string prog) (%finish-lazy-activation id #t))))))

(define (define-command! name doc proc
                         #:repeatable    [repeatable    #f]
                         #:inline-output [inline-output #f])
  (%define-command! name doc proc repeatable inline-output))

(define (define-typed-command! name doc proc
                               #:inline-output [inline-output #f])
  (%define-typed-command! name doc proc inline-output))

(define (%dispatch-command name args)
  (let ((proc (%lookup-plugin-proc name)))
    (if proc
        (apply proc args)
        (let ((owner (%lazy-command-owner name)))
          (if owner
              (begin
                (%activate-plugin-inline owner)
                (let ((proc2 (%lookup-plugin-proc name)))
                  (if proc2 (apply proc2 args) (%call-native! name args))))
              (%call-native! name args))))))

(define (register-lsp-server! language #:command command
                                        #:args [args '()]
                                        #:root-markers [root-markers '()]
                                        #:init-options [init-options #f]
                                        #:settings [settings #f]
                                        #:env [env '()])
  (%register-lsp-server! language command args root-markers init-options settings env))

(define (lsp-request server method params callback #:allow-stale [allow-stale #f]
                                                     #:supersede [supersede #f])
  (%lsp-request server method params callback allow-stale supersede))

(define (get-option . args)
  (let ([n (length args)])
    (cond
      [(= n 1) (%get-option (car args) #f)]
      [(= n 2) (%get-option (cadr args) (car args))]
      [else (error "get-option: expected (get-option key) or (get-option bid key)")])))

(define (debounce ms proc)
  (let ((pending (box #f)))
    (lambda args
      (let ((prev (unbox pending)))
        (when prev (cancel-timer! prev)))
      (let ((my-id (box #f)))
        (set-box! my-id
          (after ms (lambda ()
                      (when (equal? (unbox pending) (unbox my-id))
                        (set-box! pending #f))
                      (apply proc args))))
        (set-box! pending (unbox my-id))))))

(define (debounce-by ms proc)
  (let ((pending (box (hash))))
    (lambda args
      (let* ((key (car args))
             (table (unbox pending)))
        (when (hash-contains? table key)
          (cancel-timer! (hash-ref table key)))
        (let ((my-id (box #f)))
          (set-box! my-id
            (after ms (lambda ()
                        (let ((table (unbox pending)))
                          (when (and (hash-contains? table key)
                                     (equal? (hash-ref table key) (unbox my-id)))
                            (set-box! pending (hash-remove table key))))
                        (apply proc args))))
          (set-box! pending (hash-insert (unbox pending) key (unbox my-id))))))))

(define (diagnostics-for-buffer bid #:severity [severity #f] #:range [range #f])
  (%diagnostics-for-buffer bid severity range))

(define (buffer-lines bid #:start [start #f] #:end [end #f])
  (%buffer-lines bid start end))

(define (apply-text-edits! bid edits #:expect-generation [gen #f])
  (%apply-text-edits! bid edits gen))

(define (apply-workspace-edit! wsedit)
  (let ((n (%apply-workspace-edit! wsedit)))
    (log! 'info (to-string n " buffers modified — :wa writes all"))
    n))

(define (prompt! label on-confirm #:prefill [prefill ""])
  (%prompt! label prefill on-confirm))

(define (completion-begin! bid items #:incomplete [incomplete #f])
  (%completion-begin! bid items incomplete))

(define (run-inline-output! cmd args #:cwd [cwd #f])
  (let ([code (%run-inline-output! cmd args cwd)])
    (unless (= code 0)
      (error (string-append cmd ": failed (exit " (number->string code) ")")))))

(define (show-popup! text #:anchor [anchor 'cursor] #:kind [kind 'sticky] #:lang [lang #f])
  (%show-popup! text anchor kind lang))

(define (picker! items on-select #:prompt [prompt ""] #:pending [pending #f]
                                  #:query [query ""] #:truncate [truncate 'head])
  (%picker! items on-select prompt pending query truncate))

;; The `'(0)` default `picker-source-spawn!` and `live-picker!` both need for
;; `#:ok-exit-codes` — one literal, so the two keyword defaults can't drift.
(define %picker-source-default-ok-exit-codes '(0))

(define (picker-source-spawn! token cmd args #:cwd [cwd #f] #:nul [nul #f]
                                              #:ok-exit-codes [ok-exit-codes %picker-source-default-ok-exit-codes])
  (%picker-source-spawn! token cmd args cwd nul ok-exit-codes))

(define (live-picker! on-select #:command command
                       #:prompt [prompt ""] #:query [query ""]
                       #:debounce-ms [debounce-ms 150]
                       #:cwd [cwd #f] #:nul [nul #f]
                       #:ok-exit-codes [ok-exit-codes %picker-source-default-ok-exit-codes]
                       #:truncate [truncate 'head])
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
         ;; Cleanup-then-reraise around the debounced respawn only — never
         ;; around `spawn-for`'s direct call below for a non-empty seed
         ;; `#:query`, which runs synchronously inside whatever call stack
         ;; invoked `live-picker!` and may already be inside a caller's own
         ;; `with-handler` (see `open_live_picker`'s doc for why nesting
         ;; that pattern corrupts Steel's VM). The debounced call has no
         ;; such caller: it's dispatched fresh by the timer wheel, so a
         ;; `#:command` raise here — a bad builder, or `picker-source-spawn!`
         ;; itself failing to spawn — can't otherwise reach `picker-replace!`,
         ;; leaving the previous pattern's rows stranded under a permanently
         ;; "in flight" marker (`PickerSession::requery_armed` in
         ;; `hume-editor::editor::picker`, only ever cleared by a `replace`).
         [respawn (debounce debounce-ms
                    (lambda (token q)
                      (with-handler
                        (lambda (e) (picker-replace! token '()) (raise-error e))
                        (spawn-for token q))))]
         [token (%live-picker! on-select prompt query
                  (lambda (token q)
                    (picker-source-stop! token)
                    (respawn token q))
                  truncate)])
    (unless (equal? query "")
      (spawn-for token query))
    token))

(define (picker-close! #:token [token #f])
  (%picker-close! token))

(define-syntax call!
  (syntax-rules ()
    ((_ name args ...)
     (%dispatch-command name (list args ...)))))

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
