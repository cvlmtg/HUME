(require-builtin steel/meta as hm.)

(define (declare-plugin name #:commands  [commands  '()]
                             #:events    [events    '()]
                             #:languages [languages '()]
                             #:config    [config    (hash)])
  (if (and (null? commands) (null? events) (null? languages))
      (let ((prog (%begin-manifest-declare! name config)))
        (when prog
          (with-handler
            (lambda (e) (%finish-manifest-declare! name #f) (raise-error e))
            (begin (hm.eval-string prog) (%finish-manifest-declare! name #t)))))
      (%declare-plugin! name commands events languages config)))

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

;; (get-option key) / (get-option bid key). Rest-only parameter list, not a
;; mixed fixed-plus-rest one: a 2+-positional call site compiled inside a
;; required module hits a steel-core 0.8.2 limitation with mixed lists (see
;; builtins/io.rs's module doc), and plugin bodies are required modules.
(define (get-option . args)
  (let ([n (length args)])
    (cond
      [(= n 1) (%get-option (car args) #f)]
      [(= n 2) (%get-option (cadr args) (car args))]
      [else (error "get-option: expected (get-option key) or (get-option bid key)")])))

;; Each armed timer only clears/consumes `pending`'s slot if it's still the
;; entry stored there when it fires — checked via `my-id`, a box the timer's
;; own closure captures so it can compare "am I still the current one" at
;; fire time. Without this: two calls close enough together that the first
;; timer is already popped-and-queued (due, but not yet *run*) when the
;; second call's `cancel-timer!` targets it — a no-op, the id no longer
;; exists in the wheel — leave the second call's freshly armed timer's id
;; written into `pending`. The first timer's queued call then runs anyway
;; (it was already dequeued, nothing retroactively cancels that) and
;; unconditionally clears `pending` on the way out, wiping the second
;; timer's id out from under it — orphaned, no longer cancellable by any
;; future call, but still ticking: it fires later regardless, on its own
;; original schedule, sending a stray duplicate. Racing two calls into the
;; same fixpoint drain is exactly what a merged, always-draining `settle()`
;; makes routine (hume-editor's C4), so this is no longer a corner case.
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

;; debounce-by — like `debounce`, but keyed per first-argument value instead
;; of one shared pending timer: a call keyed `k1` never cancels a call keyed
;; `k2`. Same trailing-edge semantics per key, and the same current-entry
;; check `debounce` uses (see its comment) against races within one key.
;; Relies on the calling convention already used everywhere `debounce` wraps
;; a single-bid handler (`(lambda (bid) ...)`) — the key is `(car args)`,
;; not a separate keyfn argument, so swapping `debounce` for `debounce-by`
;; at an existing call site needs no other change.
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

(define (picker! items on-select #:prompt [prompt ""] #:pending [pending #f])
  (%picker! items on-select prompt pending))

(define (picker-source-spawn! token cmd args #:cwd [cwd #f] #:nul [nul #f])
  (%picker-source-spawn! token cmd args cwd nul))

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
