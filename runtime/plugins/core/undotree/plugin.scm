;;; core:undotree — plugin.scm. See README.md.

(require "render.scm")

;; ── Session ──────────────────────────────────────────────────────────────────
;; See README.md's "Session".

(define undotree/*session* #f)

;; ── Revision diff ────────────────────────────────────────────────────────────
;; See README.md's "Revision diff".

(define undotree/*source* "undotree")
(define undotree/diff-command "git-diff/render-diff")
(define undotree/release-command "git-diff/release-diff")

(define (undotree/diff-available?)
  (and (command-exists? undotree/diff-command) (command-exists? undotree/release-command)))

(define (undotree/draw-diff! pane node)
  (let ([parent (hash-ref node 'parent)])
    (call! undotree/diff-command pane undotree/*source*
           (if parent (hash-ref (buffer-revision-diff pane parent) 'hunks) '()))))

(define (undotree/clear-diff! session)
  (let ([pane (hash-ref session 'pane)])
    (when (and (hash-ref session 'drawn) (buffer-live? pane))
      (call! undotree/release-command pane undotree/*source*))))

(define (undotree/end-session!)
  (let ([session undotree/*session*])
    (set! undotree/*session* #f)
    (when session
      (cancel-timer! (hash-ref session 'timer))
      (undotree/clear-diff! session))))

(define (undotree/session-drawer? drawer)
  (and undotree/*session* (equal? (hash-ref undotree/*session* 'drawer) drawer)))

(define (undotree/shows-buffer? session pane)
  (and session (equal? (buffer-key pane) (buffer-key (hash-ref session 'pane)))))

;;; Makes `drawer`, which shows `pane`'s freshly rendered tree, the session.
;;; The session is stored before the diff is drawn, so a raising renderer
;;; leaves it matching the drawer.
(define (undotree/commit! drawer pane rendered)
  (let* ([previous undotree/*session*]
         [node (hash-ref rendered 'current-node)]
         [revision (and (undotree/diff-available?) (hash-ref node 'id))]
         [same-buffer? (undotree/shows-buffer? previous pane)]
         [timer (after! (* (hash-ref rendered 'next-change-secs) 1000)
                        (lambda ()
                          (when (undotree/session-drawer? drawer)
                            (undotree/show!))))])
    (set! undotree/*session*
          (hash 'drawer drawer 'pane pane 'drawn revision 'timer timer))
    (when previous
      (cancel-timer! (hash-ref previous 'timer)))
    (when (and previous (not same-buffer?))
      (undotree/clear-diff! previous))
    (when (and revision
               (not (and same-buffer? (equal? revision (hash-ref previous 'drawn)))))
      (undotree/draw-diff! pane node))))

;; ── Drawer ───────────────────────────────────────────────────────────────────

(define (undotree/on-select node drawer)
  (when (undotree/session-drawer? drawer)
    (if node
        (undotree/jump! node)
        (undotree/end-session!))))

;;; Re-renders the focused pane's tree into the open drawer. See README.md's "Refresh".
(define (undotree/show!)
  (let* ([session undotree/*session*]
         [pane (focused-pane)]
         [drawer (hash-ref session 'drawer)]
         [selection (drawer-selection drawer)])
    (if (not selection)
        (undotree/end-session!)
        (let* ([rendered (undotree/render (buffer-undo-tree pane))]
               [kept (and (undotree/shows-buffer? session pane)
                          ((hash-ref rendered 'row-of) (hash-ref (car selection) 'id)))]
               [highlight (or kept (hash-ref rendered 'current))])
          (if (update-drawer-list! drawer (hash-ref rendered 'nodes) undotree/on-select highlight
                                   #:render (hash-ref rendered 'render))
              (undotree/commit! drawer pane rendered)
              (undotree/end-session!))))))

(define (undotree/jump! node)
  (let ([pane (focused-pane)])
    (if (undotree/shows-buffer? undotree/*session* pane)
        (goto-revision! pane (hash-ref node 'id))
        (undotree/show!))))

(define (undotree/open! pane)
  (undotree/end-session!)
  (let* ([rendered (undotree/render (buffer-undo-tree pane))]
         [drawer (show-drawer-list! (focused-pane) (hash-ref rendered 'nodes)
                                    undotree/on-select
                                    #:selected (hash-ref rendered 'current)
                                    #:render (hash-ref rendered 'render))])
    (when drawer
      (undotree/commit! drawer pane rendered)
      (unless (undotree/diff-available?)
        (log! 'info "undotree: load core:git-diff to see what each revision changed")))))

(define (undotree/toggle! pane)
  (let ([session undotree/*session*])
    (if (and session (drawer-selection (hash-ref session 'drawer)))
        (begin
          (close-drawer! (hash-ref session 'drawer))
          (undotree/end-session!))
        (undotree/open! pane))))

;; ── Commands ─────────────────────────────────────────────────────────────────

(define undotree/toggle-doc
  "Show or hide the focused buffer's undo tree in the bottom drawer. Enter on a row jumps to that revision.")

(define-command! "undotree/toggle" undotree/toggle-doc undotree/toggle!)

(define-typed-command! "undotree" undotree/toggle-doc undotree/toggle!)

;; ── Hooks ────────────────────────────────────────────────────────────────────

(register-hook! 'on-undo-history-changed
  (lambda (pane)
    (when (undotree/shows-buffer? undotree/*session* pane)
      (undotree/show!))))

(register-hook! 'on-buffer-enter
  (lambda (pane)
    (when undotree/*session*
      (undotree/show!))))
