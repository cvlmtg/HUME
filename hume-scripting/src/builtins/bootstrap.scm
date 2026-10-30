(require-builtin steel/meta as hm.)

(define (declare-plugin! name #:entry          [entry          #f]
                             #:commands       [commands       '()]
                             #:typed-commands [typed-commands '()]
                             #:events         [events         '()]
                             #:languages      [languages      '()]
                             #:config         [config         #f])
  (when config
    (error "declare-plugin!: takes no #:config; pass it with (load-plugin! name #:config ...)"))
  (%declare-plugin! name entry commands typed-commands events languages))

(define (load-plugin! name #:entry [entry #f] #:config [config (hash)] #:ref [ref #f])
  (when entry
    (error "load-plugin!: takes no #:entry; a plugin's manifest.scm declares its entries"))
  (let ((next (%load-plugin! name config ref)))
    (cond
      ((string? next)
       (with-handler
         (lambda (e) (%finish-manifest-load! name e))
         (begin (hm.eval-string next) (%finish-manifest-load! name #f))))
      (next (%activate-plugin-inline! name #f)))))

(define (%activate-plugin-inline! plugin entry)
  (let ((prog (%begin-lazy-activation! plugin entry)))
    (when prog
      (with-handler
        (lambda (e) (%finish-lazy-activation! e))
        (begin (hm.eval-string prog) (%finish-lazy-activation! #f))))))

(define (define-command! name doc proc
                         #:repeatable    [repeatable    #f]
                         #:inline-output [inline-output #f])
  (%define-command! name doc proc repeatable inline-output))

(define (define-typed-command! name doc proc
                               #:inline-output [inline-output #f]
                               #:complete [complete #f])
  (%define-typed-command! name doc proc inline-output complete))

(define (%apply-command! entered name args)
  (let ((depth (%arm-inline-output! name)))
    (dynamic-wind (lambda () #t)
                  (lambda () (apply (car entered) args))
                  (lambda ()
                    (when depth (%restore-inline-output! depth))
                    (%leave-command! (cadr entered))))))

(define (%call-held-handler handler hold args)
  (apply handler (append args (list (lambda () (%held-done hold))))))

(define (%dispatch-command! name args)
  (let ((entered (%enter-command! name)))
    (if entered
        (%apply-command! entered name args)
        (let ((owner (%lazy-command-owner name)))
          (if owner
              (let ((plugin (car owner)) (entry (cadr owner)) (label (caddr owner)))
                (%activate-plugin-inline! plugin entry)
                (let ((entered2 (%enter-command! name)))
                  (cond
                    (entered2 (%apply-command! entered2 name args))
                    ((%entry-loaded? plugin entry)
                     (error (string-append "'" name "': plugin '" label
                                           "' loaded but did not define it")))
                    (else
                     (error (string-append "'" name "' unavailable: plugin '" label
                                           "' failed to load"))))))
              (%call-native! name args))))))

(define (register-lsp-server! name #:command command
                                    #:args [args '()]
                                    #:init-options [init-options #f]
                                    #:settings [settings #f]
                                    #:env [env '()])
  (%register-lsp-server! name command args init-options settings env))

(define (lsp-request! pane method params callback #:feature [feature #f]
                                                   #:to [to #f]
                                                   #:unavailable [unavailable 'error]
                                                   #:allow-stale [allow-stale #f]
                                                   #:supersede [supersede #f]
                                                   #:require-focus [require-focus #f]
                                                   #:tracked [tracked #f])
  (%lsp-request! pane method params callback feature to unavailable
                 allow-stale supersede require-focus tracked))

(define (lsp-request-all! pane method params callback #:feature [feature #f]
                                                       #:unavailable [unavailable 'error]
                                                       #:allow-stale [allow-stale #f]
                                                       #:supersede [supersede #f]
                                                       #:require-focus [require-focus #f]
                                                       #:tracked [tracked #f])
  (%lsp-request-all! pane method params callback feature unavailable
                     allow-stale supersede require-focus tracked))

(define (lsp-notify! pane method params #:feature [feature #f] #:to [to #f])
  (%lsp-notify! pane method params feature to))

(define (lsp-servers pane #:feature [feature #f] #:method [method #f])
  (%lsp-servers pane feature method))

(define (lsp-capability server #:feature [feature #f] #:method [method #f])
  (%lsp-capability server feature method))

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

(define (diagnostic-counts pane #:severity [severity #f])
  (%diagnostic-counts pane severity))

(define (set-eol-text! pane source lines #:hide-on-insert-line [hide #f])
  (%set-eol-text! pane source lines hide))

(define (buffer-lines pane #:range [range #f])
  (%buffer-lines pane range))

(define (bind-key! mode key cmd #:keymap [keymap #f])
  (%bind-key! mode key cmd keymap))

(define (bind-key-extend! mode key cmd #:keymap [keymap #f])
  (%bind-key-extend! mode key cmd keymap))

(define (unbind-key! mode key #:keymap [keymap #f])
  (%unbind-key! mode key keymap))

(define (buffer-display-path pane #:relative [relative #f])
  (%buffer-display-path pane relative))

(define (path->display path #:relative [relative #f])
  (%path->display path relative))

(define (close-buffer! pane #:discard-unsaved [discard-unsaved #f])
  (%close-buffer! pane discard-unsaved))

(define (apply-text-edits! pane edits #:expect-generation [gen #f])
  (%apply-text-edits! pane edits gen))

(define (edit-text! pane edits #:expect-generation [gen #f])
  (%edit-text! pane edits gen))

(define (apply-workspace-edit! pane wsedit #:expect-generation [gen #f])
  (%apply-workspace-edit! pane wsedit gen))

(define (prompt! pane label on-confirm #:prefill [prefill ""])
  (%prompt! pane label prefill on-confirm))

(define (register-completion-source! name proc #:target target
                                               #:match [match-kind 'fuzzy] #:priority [priority 0]
                                               #:resolve [resolve #f]
                                               #:token-chars [token-chars ""]
                                               #:cancel [cancel #f])
  (%register-completion-source! name proc target match-kind priority resolve token-chars cancel))

(define (completion-emit! id items #:incomplete [incomplete #f] #:more [more #f]
                                   #:span [span #f] #:prefix [prefix #t])
  (%completion-emit! id items incomplete more span prefix))

(define (spawn-async! cmd args callback #:cwd [cwd #f])
  (%spawn-async! cmd args cwd callback))

(define (run-capture! cmd args #:cwd [cwd #f])
  (%run-capture! cmd args cwd))

(define (run-inline-output! cmd args #:cwd [cwd #f] #:env [env '()])
  (%run-inline-output! cmd args cwd env))

(define (show-popup! pane text #:anchor [anchor 'cursor] #:kind [kind 'sticky] #:lang [lang #f])
  (%show-popup! pane text anchor kind lang))

(define (show-drawer-list! pane items on-select #:selected [selected 0] #:render [render #f])
  (%show-drawer-list! pane items on-select selected render))

(define (update-drawer-list! token items on-select selected #:render [render #f])
  (%update-drawer-list! token items on-select selected render))

(define (picker! pane items keymap #:prompt [prompt ""] #:pending [pending #f]
                                    #:query [query ""] #:truncate [truncate 'head]
                                    #:on-close [on-close #f])
  (%picker! pane items keymap prompt pending query truncate on-close))

(define (picker-move! token count #:by [by 'row])
  (%picker-move! token count by))

(define %picker-source-default-ok-exit-codes '(0))

(define (picker-source-spawn! token cmd args #:cwd [cwd #f] #:nul [nul #f]
                                              #:ok-exit-codes [ok-exit-codes %picker-source-default-ok-exit-codes])
  (%picker-source-spawn! token cmd args cwd nul ok-exit-codes))

(define (picker-command-source argv-for #:cwd [cwd #f] #:nul [nul #f]
                                         #:ok-exit-codes [ok-exit-codes %picker-source-default-ok-exit-codes])
  (unless (%callable? argv-for 1)
    (error "picker-command-source: argv-for must be a procedure of one argument (the query)"))
  (lambda (token q)
    (let ([argv (argv-for q)])
      (if argv
          (begin
            (unless (and (list? argv) (not (null? argv)) (string? (car argv)))
              (error "picker-command-source: argv-for must return #f or a non-empty list of strings (argv)"))
            (picker-source-spawn! token (car argv) (cdr argv)
                                  #:cwd cwd #:nul nul #:ok-exit-codes ok-exit-codes))
          (picker-replace! token '())))))

(define (live-picker! pane keymap #:source source
                       #:prompt [prompt ""] #:query [query ""]
                       #:debounce-ms [debounce-ms 150]
                       #:truncate [truncate 'head]
                       #:on-close [on-close #f])
  (unless (%callable? source 2)
    (error "live-picker!: #:source must be a procedure of two arguments (the picker token and the query)"))
  (unless (and (integer? debounce-ms) (>= debounce-ms 0))
    (error "live-picker!: #:debounce-ms must be a non-negative integer"))
  (let* ([requery (debounce debounce-ms
                    (lambda (token q)
                      (with-handler
                        (lambda (e) (picker-replace! token '()) (raise-error e))
                        (source token q))))]
         [token (%live-picker! pane keymap prompt query
                  (lambda (token q)
                    (picker-source-stop! token)
                    (requery token q))
                  truncate on-close)])
    (unless (equal? query "")
      (source token query))
    token))


(define-syntax call!
  (syntax-rules ()
    ((_ name args ...)
     (%dispatch-command! name (list args ...)))))

(define (command-exists? name)
  (or (%command-callable? name)
      (with-handler (lambda (e) #f)
        (procedure? (eval (string->symbol name))))))

(define (%language-data-all? ok? items)
  (or (null? items) (and (ok? (car items)) (%language-data-all? ok? (cdr items)))))

(define (stems-with-suffix dir suffix)
  (if (not (path-exists? dir))
      '()
      (let ((slen (string-length suffix)))
        (sort (map (lambda (f) (substring f 0 (- (string-length f) slen)))
                   (filter (lambda (f) (and (ends-with? f suffix)
                                            (> (string-length f) slen)))
                           (map file-name (read-dir dir))))
              string<?))))

(define (%language-data-dir)
  (let ((runtime (runtime-dir)))
    (and runtime (path-join runtime "scheme" "languages"))))

(define (data-file-names dir)
  (stems-with-suffix dir ".scm"))

(define (data-file dir name)
  (let ((file (string-append name ".scm")))
    (and (path-exists? dir)
         (member file (map file-name (read-dir dir)))
         (path-join dir file))))

(define (read-data-file path)
  (let* ((text (call-with-input-file path read-port-to-string))
         (forms (with-handler (lambda (err) (error (string-append path ": " (to-string err))))
                              (hm.read! text))))
    (unless (= (length forms) 1)
      (error (string-append path ": expected one top-level form, found "
                            (number->string (length forms)))))
    (car forms)))

(define (%language-data-file name)
  (let ((dir (%language-data-dir)))
    (and dir (data-file dir name))))

(define (%language-data-strings? items)
  (and (list? items) (%language-data-all? string? items)))

(define (%language-data-string-pair? item)
  (and (pair? item) (string? (car item)) (string? (cdr item))))

(define (%language-data-entry-problem key body)
  (cond
    ((equal? key 'comment-tokens)
     (and (not (%language-data-strings? body)) "comment-tokens must be strings"))
    ((equal? key 'block-comment-tokens)
     (and (not (and (list? body) (%language-data-all? %language-data-string-pair? body)))
          "block-comment-tokens must be (start . end) string pairs"))
    ((equal? key 'language-id)
     (and (not (string? body)) "language-id must be (language-id . \"id\")"))
    ((equal? key 'roots)
     (and (not (%language-data-strings? body)) "roots must be strings"))
    (else (string-append "unknown key " (symbol->string key)))))

(define (%check-language-data path entries)
  (define (fail reason) (error (string-append path ": " reason)))
  (unless (list? entries) (fail "expected a list of entries"))
  (let loop ((rest entries) (seen '()))
    (unless (null? rest)
      (let ((entry (car rest)))
        (unless (and (pair? entry) (symbol? (car entry)))
          (fail "an entry must start with a key symbol"))
        (let* ((key (car entry))
               (problem (%language-data-entry-problem key (cdr entry))))
          (when (member key seen)
            (fail (string-append (symbol->string key) " appears twice")))
          (when problem (fail problem))
          (loop (cdr rest) (cons key seen))))))
  entries)

(define (%read-language-data name)
  (let ((path (%language-data-file name)))
    (and path (%check-language-data path (read-data-file path)))))

(define (%bundled-language-config name)
  (let ((data (%read-language-data name)))
    (and data
         (let loop ((rest data) (fields (hash)))
           (if (null? rest)
               fields
               (loop (cdr rest)
                     (hash-insert fields (car (car rest)) (cdr (car rest)))))))))

(define (bundled-languages)
  (let ((dir (%language-data-dir)))
    (if dir (data-file-names dir) '())))
