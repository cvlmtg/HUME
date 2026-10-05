;;; core:undotree — plugin.scm. See README.md.

(require "render.scm")

;; ── Session ──────────────────────────────────────────────────────────────────
;; See README.md's "Session".

(define undotree/*session* #f)

;; See README.md's "Refresh".
(define undotree/*age-timer* #f)

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
           (if parent (buffer-revision-diff pane parent) '()))))

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

(define (undotree/session-drawer? drawer)
  (and undotree/*session* (equal? (hash-ref undotree/*session* 'drawer) drawer)))

(define (undotree/session-buffer? pane)
  (equal? (buffer-key pane) (buffer-key (hash-ref undotree/*session* 'pane))))

(define (undotree/arm-age-timer! drawer secs)
  (when undotree/*age-timer*
    (cancel-timer! undotree/*age-timer*))
  (set! undotree/*age-timer*
        (after! (* secs 1000)
                (lambda ()
                  (when (undotree/session-drawer? drawer)
                    (undotree/show!))))))

;;; Makes `drawer`, which shows `pane`'s freshly rendered tree, the session.
;;; The session is stored before the diff is drawn, so a raising renderer
;;; leaves it matching the drawer.
(define (undotree/commit! drawer pane rendered)
  (let* ([previous undotree/*session*]
         [node (hash-ref rendered 'current-node)]
         [revision (and (undotree/diff-available?) (hash-ref node 'id))]
         [same-buffer? (and previous
                            (equal? (buffer-key pane) (buffer-key (hash-ref previous 'pane))))])
    (set! undotree/*session*
          (hash 'drawer drawer 'pane pane 'nodes (hash-ref rendered 'nodes)
                'drawn revision))
    (undotree/arm-age-timer! drawer (hash-ref rendered 'next-change-secs))
    (when (and previous (not same-buffer?))
      (undotree/clear-diff! (hash-ref previous 'pane)))
    (when (and revision
               (not (and same-buffer? (equal? revision (hash-ref previous 'drawn)))))
      (undotree/draw-diff! pane node))))

;; ── Drawer ───────────────────────────────────────────────────────────────────

(define (undotree/row-id nodes idx)
  (hash-ref (list-ref nodes idx) 'id))

(define (undotree/on-select key nodes)
  (lambda (idx drawer)
    (when (undotree/session-drawer? drawer)
      (if idx
          (undotree/jump! key (list-ref nodes idx))
          (undotree/end-session!)))))

;;; Re-renders the focused pane's tree into the open drawer. See README.md's "Refresh".
(define (undotree/show!)
  (let* ([session undotree/*session*]
         [pane (focused-pane)]
         [drawer (hash-ref session 'drawer)]
         [selected (drawer-selected-index drawer)])
    (if (not selected)
        (undotree/end-session!)
        (let* ([rendered (undotree/render (buffer-undo-tree pane))]
               [nodes (hash-ref rendered 'nodes)]
               [kept (and (undotree/session-buffer? pane)
                          (let ([wanted (undotree/row-id (hash-ref session 'nodes) selected)])
                            (undotree/row-index (lambda (node) (equal? (hash-ref node 'id) wanted))
                                                nodes)))]
               [highlight (or kept (hash-ref rendered 'current))])
          (if (update-drawer-list! drawer nodes (undotree/on-select (buffer-key pane) nodes) highlight
                                   #:render (hash-ref rendered 'render))
              (undotree/commit! drawer pane rendered)
              (undotree/end-session!))))))

(define (undotree/jump! key node)
  (let ([pane (focused-pane)])
    (if (equal? (buffer-key pane) key)
        (goto-revision! pane (hash-ref node 'id))
        (undotree/show!))))

(define (undotree/open! pane)
  (undotree/end-session!)
  (let* ([rendered (undotree/render (buffer-undo-tree pane))]
         [drawer (show-drawer-list! (focused-pane) (hash-ref rendered 'nodes)
                                    (undotree/on-select (buffer-key pane) (hash-ref rendered 'nodes))
                                    #:selected (hash-ref rendered 'current)
                                    #:render (hash-ref rendered 'render))])
    (when drawer
      (undotree/commit! drawer pane rendered)
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

(define undotree/toggle-doc
  "Show or hide the focused buffer's undo tree in the bottom drawer. Enter on a row jumps to that revision.")

(define-command! "toggle-undotree"
  undotree/toggle-doc
  (lambda (pane) (undotree/toggle! pane)))

(define-typed-command! "undotree"
  undotree/toggle-doc
  (lambda (pane) (undotree/toggle! pane)))

;; ── Hooks ────────────────────────────────────────────────────────────────────

(register-hook! 'on-undo-history-changed
  (lambda (pane)
    (when (and undotree/*session* (undotree/session-buffer? pane))
      (undotree/show!))))

(register-hook! 'on-buffer-enter
  (lambda (pane)
    (when undotree/*session*
      (undotree/show!))))
