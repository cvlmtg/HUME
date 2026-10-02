;;; core:undotree — plugin.scm. See README.md.

(require "render.scm")

;; ── Session ──────────────────────────────────────────────────────────────────
;; See README.md's "Session".

(define undotree/*session* #f)
(define undotree/*next-id* 0)

;; The row ages are relative to the render, so an idle drawer re-renders on
;; this period. See README.md's "Refresh".
(define undotree/age-refresh-ms 60000)
(define undotree/*age-timer* #f)

(define (undotree/end-session!)
  (when undotree/*age-timer*
    (cancel-timer! undotree/*age-timer*)
    (set! undotree/*age-timer* #f))
  (set! undotree/*session* #f))

(define (undotree/session-is? id)
  (and undotree/*session* (equal? (hash-ref undotree/*session* 'id) id)))

;; ── Drawer ───────────────────────────────────────────────────────────────────

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
               [ids (hash-ref rendered 'ids)]
               [same-buffer? (equal? (buffer-key pane) (hash-ref session 'key))]
               [kept (and same-buffer?
                          (let ([wanted (list-ref (hash-ref session 'ids) selected)])
                            (undotree/row-index (lambda (id) (equal? id wanted)) ids 0)))]
               [highlight (or kept (hash-ref rendered 'current))])
          (if (update-drawer-list! drawer (hash-ref rendered 'rows)
                                   (undotree/on-select (hash-ref session 'id)) highlight)
              (set! undotree/*session*
                    (hash 'id (hash-ref session 'id) 'drawer drawer 'pane pane
                          'key (buffer-key pane) 'ids ids))
              (undotree/end-session!))))))

(define (undotree/jump! idx)
  (let* ([session undotree/*session*]
         [pane (hash-ref session 'pane)])
    (if (pane-live? pane)
        (goto-revision! pane (list-ref (hash-ref session 'ids) idx))
        (undotree/show! (focused-pane)))))

(define (undotree/arm-age-timer! id)
  (set! undotree/*age-timer*
        (after! undotree/age-refresh-ms
                (lambda ()
                  (when (undotree/session-is? id)
                    (let ([pane (hash-ref undotree/*session* 'pane)])
                      (undotree/show! (if (pane-live? pane) pane (focused-pane))))
                    (when (undotree/session-is? id)
                      (undotree/arm-age-timer! id)))))))

(define (undotree/open! pane)
  (undotree/end-session!)
  (set! undotree/*next-id* (+ undotree/*next-id* 1))
  (let* ([id undotree/*next-id*]
         [rendered (undotree/render (buffer-undo-tree pane))]
         [rows (hash-ref rendered 'rows)]
         [drawer (show-drawer-list! (focused-pane) rows (undotree/on-select id))])
    (when drawer
      (set! undotree/*session*
            (hash 'id id 'drawer drawer 'pane pane 'key (buffer-key pane)
                  'ids (hash-ref rendered 'ids)))
      (update-drawer-list! drawer rows (undotree/on-select id) (hash-ref rendered 'current))
      (undotree/arm-age-timer! id))))

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

(define undotree/refresh-later
  (debounce 150
            (lambda ()
              (when undotree/*session*
                (let ([pane (hash-ref undotree/*session* 'pane)])
                  (undotree/show! (if (pane-live? pane) pane (focused-pane))))))))

(register-hook! 'on-undo-history-changed
  (lambda (pane)
    (when (and undotree/*session*
               (equal? (buffer-key pane) (hash-ref undotree/*session* 'key)))
      (undotree/refresh-later))))

(register-hook! 'on-buffer-enter
  (lambda (pane)
    (when (and undotree/*session*
               (not (equal? (buffer-key pane) (hash-ref undotree/*session* 'key))))
      (undotree/show! pane))))
