;;; core:undotree — plugin.scm. See README.md.

(require "render.scm")

;; ── Session ──────────────────────────────────────────────────────────────────
;; See README.md's "Session".

(define undotree/*session* #f)
(define undotree/*next-id* 0)

;; See README.md's "Refresh".
(define undotree/age-refresh-ms 60000)
(define undotree/*age-timer* #f)

;; ── Revision diff ────────────────────────────────────────────────────────────
;; See README.md's "Revision diff".

(define undotree/*source* "undotree")
(define undotree/diff-command "git-diff/render-diff")
(define undotree/release-command "git-diff/release-diff")

(define (undotree/diff-available?)
  (and (command-exists? undotree/diff-command) (command-exists? undotree/release-command)))

;;; Returns the marker now on screen, `#f` without a renderer — see README.md's "Revision diff".
(define (undotree/draw-diff! pane node drawn)
  (if (not (undotree/diff-available?))
      #f
      (let* ([marker (cons (buffer-key pane) (hash-ref node 'id))]
             [parent (hash-ref node 'parent)])
        (unless (equal? marker drawn)
          (call! undotree/diff-command pane undotree/*source*
                 (if parent (buffer-revision-diff pane parent) '())))
        marker)))

(define (undotree/clear-diff! pane)
  (when (and (undotree/diff-available?) (buffer-live? pane))
    (call! undotree/release-command pane undotree/*source*)))

(define (undotree/end-session!)
  (let ([session undotree/*session*])
    (when undotree/*age-timer*
      (cancel-timer! undotree/*age-timer*)
      (set! undotree/*age-timer* #f))
    (set! undotree/*session* #f)
    (when session
      (undotree/clear-diff! (hash-ref session 'pane)))))

(define (undotree/session-is? id)
  (and undotree/*session* (equal? (hash-ref undotree/*session* 'id) id)))

(define (undotree/start-session! id drawer pane nodes node drawn)
  (set! undotree/*session*
        (hash 'id id 'drawer drawer 'pane pane 'key (buffer-key pane) 'nodes nodes
              'drawn (undotree/draw-diff! pane node drawn))))

(define (undotree/session-pane)
  (let ([pane (hash-ref undotree/*session* 'pane)])
    (if (pane-live? pane) pane (focused-pane))))

;; ── Drawer ───────────────────────────────────────────────────────────────────

(define (undotree/row-id nodes idx)
  (hash-ref (list-ref nodes idx) 'id))

(define (undotree/on-select id)
  (lambda (idx)
    (when (undotree/session-is? id)
      (if idx (undotree/jump! idx) (undotree/end-session!)))))

;;; Re-renders `pane`'s tree into the open drawer. See README.md's "Refresh".
(define (undotree/show! pane)
  (let* ([session undotree/*session*]
         [drawer (hash-ref session 'drawer)]
         [selected (drawer-selected-index drawer)])
    (if (not selected)
        (undotree/end-session!)
        (let* ([rendered (undotree/render (buffer-undo-tree pane))]
               [nodes (hash-ref rendered 'nodes)]
               [same-buffer? (equal? (buffer-key pane) (hash-ref session 'key))]
               [kept (and same-buffer?
                          (let ([wanted (undotree/row-id (hash-ref session 'nodes) selected)])
                            (undotree/row-index (lambda (node) (equal? (hash-ref node 'id) wanted))
                                                nodes)))]
               [highlight (or kept (hash-ref rendered 'current))])
          (if (update-drawer-list! drawer nodes (undotree/on-select (hash-ref session 'id)) highlight
                                   #:render (hash-ref rendered 'render))
              (begin
                (unless same-buffer? (undotree/clear-diff! (hash-ref session 'pane)))
                (undotree/start-session! (hash-ref session 'id) drawer pane nodes
                                         (hash-ref rendered 'current-node)
                                         (hash-ref session 'drawn)))
              (undotree/end-session!))))))

(define (undotree/jump! idx)
  (let* ([session undotree/*session*]
         [pane (hash-ref session 'pane)])
    (if (pane-live? pane)
        (goto-revision! pane (undotree/row-id (hash-ref session 'nodes) idx))
        (undotree/show! (focused-pane)))))

(define (undotree/arm-age-timer! id)
  (set! undotree/*age-timer*
        (after! undotree/age-refresh-ms
                (lambda ()
                  (when (undotree/session-is? id)
                    (undotree/show! (undotree/session-pane))
                    (when (undotree/session-is? id)
                      (undotree/arm-age-timer! id)))))))

(define (undotree/open! pane)
  (undotree/end-session!)
  (set! undotree/*next-id* (+ undotree/*next-id* 1))
  (let* ([id undotree/*next-id*]
         [rendered (undotree/render (buffer-undo-tree pane))]
         [drawer (show-drawer-list! (focused-pane) (hash-ref rendered 'nodes)
                                    (undotree/on-select id)
                                    #:selected (hash-ref rendered 'current)
                                    #:render (hash-ref rendered 'render))])
    (when drawer
      (undotree/start-session! id drawer pane (hash-ref rendered 'nodes)
                               (hash-ref rendered 'current-node) #f)
      (undotree/arm-age-timer! id)
      (unless (undotree/diff-available?)
        (log! 'info "undotree: load core:git-diff to see what each revision changed")))))

(define (undotree/toggle! pane)
  (let ([session undotree/*session*])
    (if (and session (drawer-selected-index (hash-ref session 'drawer)))
        (begin
          (close-drawer! (hash-ref session 'drawer))
          (undotree/end-session!))
        (undotree/open! pane))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define-command! "toggle-undotree"
  "Show or hide the focused buffer's undo tree in the bottom drawer. Enter on a row jumps to that revision."
  (lambda (pane) (undotree/toggle! pane)))

(define-typed-command! "undotree"
  "Show or hide the focused buffer's undo tree in the bottom drawer. Enter on a row jumps to that revision."
  (lambda (pane arg) (undotree/toggle! pane)))

;; ── Hooks ────────────────────────────────────────────────────────────────────

(register-hook! 'on-undo-history-changed
  (lambda (pane)
    (when (and undotree/*session*
               (equal? (buffer-key pane) (hash-ref undotree/*session* 'key)))
      (undotree/show! (undotree/session-pane)))))

(register-hook! 'on-buffer-enter
  (lambda (pane)
    (when (and undotree/*session*
               (not (equal? (buffer-key pane) (hash-ref undotree/*session* 'key))))
      (undotree/show! pane))))
