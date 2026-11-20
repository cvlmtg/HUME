;;; core:markdown — plugin.scm. See docs/architecture.md.

(require "lib/state.scm")
(require "lib/render.scm")
(require "lib/text.scm")
(require "lib/links.scm")
(require "lib/paths.scm")
(require "lib/queries.scm")
(require "lib/lists.scm")

;; ── Config ────────────────────────────────────────────────────────────────────

(define md-preview/cfg (plugin-config))
(define md-preview/enabled-default
  (call! "stdlib/config-boolean" "core:markdown" md-preview/cfg 'preview #t))
(define md-preview/rule-width
  (call! "stdlib/config-integer" "core:markdown" md-preview/cfg 'rule-width 40 1))
(define md-lists/enabled
  (call! "stdlib/config-boolean" "core:markdown" md-preview/cfg 'continue-lists #t))

;; ── Rendering ─────────────────────────────────────────────────────────────────

(define (md-preview/track! pane)
  (md-preview/track-buffer! pane md-preview/enabled-default))

(define (md-preview/render! pane)
  (md-preview/render-buffer! pane md-preview/rule-width))

(define md-preview/render-soon!
  (debounce-by 50 md-preview/render! #:key (lambda (p . _) (buffer-key p))))

(define (md-preview/warn-missing-grammars!)
  (for-each
    (lambda (name)
      (unless (language-has-grammar? name)
        (md-preview/warn-once! name
          (string-append "markdown: no " name " grammar, so its syntax shows raw;"
                         " run :plum-install-grammar " name))))
    '("markdown" "markdown.inline")))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(define md-lists/keymap "markdown/lists")

(register-hook! 'on-buffer-open
  (lambda (pane)
    (md-preview/warn-missing-grammars!)
    (attach-keymap! pane md-lists/keymap)))

(register-hook! 'on-viewport-change
  (lambda (pane range headers) (md-preview/render! pane)))

(register-hook! 'on-syntax-changed
  (lambda (pane)
    (md-preview/track! pane)
    (md-preview/note-tree! pane)
    (md-preview/render-soon! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane)
    (md-preview/remove-buffer! pane)
    (when (buffer-live? pane)
      (md-preview/clear! pane)
      (detach-keymap! pane md-lists/keymap))))

;; ── Links ─────────────────────────────────────────────────────────────────────

(register-hook! 'on-paste
  (lambda (pane pasted previous) (md-links/paste-link pane pasted previous)))

(register-completion-source! "markdown:paths" md-paths/complete!
  #:target 'buffer #:priority 20 #:token-chars "!\"$%&'*+,-./:;=?@[]^`{|}~")

(set-completion-triggers! "markdown" "markdown:paths" (list #\( #\/ #\<))

;; ── Lists ─────────────────────────────────────────────────────────────────────

(define (md-lists/groups pane spans)
  (md-preview/group
    (if (language-has-grammar? "markdown")
        (syntax-captures pane "markdown" md-lists/query
                         #:ranges (map (lambda (span) (hash 'start-line (car span) 'end-line (cdr span)))
                                       spans))
        '())))

(define (md-lists/prefix-edits pane prefixes)
  (let ([sels (buffer-selections pane)])
    (unless (= (length sels) (length prefixes))
      (error "markdown: the selection count differs from the list prefixes planned for it"))
    (call! "stdlib/concat"
      (map (lambda (sel prefix)
             (if prefix
                 (let* ([head (selection-head sel)]
                        [start (line->char-offset pane (char-offset->line pane head))]
                        [insert (hash 'start-char head 'end-char head 'text prefix)])
                   (if (< start head)
                       (list (hash 'start-char start 'end-char head 'text "") insert)
                       (list insert)))
                 '()))
           sels
           prefixes))))

;;; See docs/lists.md's "Order of edits".
(define (md-lists/enter! pane)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [heads (map selection-head (buffer-selections pane))]
         [lines (map line-of heads)])
    (if (findf (lambda (line) (md-lists/may-start-prefix? (cdr (line-info line)))) lines)
        (let* ([groups (md-lists/groups pane (map (lambda (line) (cons line (+ line 1))) lines))]
               [items (map (lambda (item head) (and item (md-lists/past-prefix? item head) item))
                           (md-lists/items lines groups line-info)
                           heads)])
          (if (null? (filter (lambda (item) (not (and item (md-lists/empty? item)))) items))
              (edit-text! pane (map md-lists/ended-edit items))
              (let ([numbers (cdr (md-lists/renumber groups (md-lists/events items 'after) line-of line-info))])
                (insert-key! pane "enter")
                (edit-text! pane (md-lists/prefix-edits pane (md-lists/prefixes items numbers))))))
        (insert-key! pane "enter"))))

;;; See docs/lists.md's "Order of edits".
(define (md-lists/open! pane below?)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [sources (md-lists/distinct
                    (map (lambda (sel)
                           (line-of (if below? (- (selection-end-char sel) 1) (selection-start-char sel))))
                         (buffer-selections pane)))]
         [groups (md-lists/groups pane (map (lambda (line) (cons line (+ line 1))) sources))]
         [items (md-lists/items sources groups line-info)]
         [numbers (cdr (md-lists/renumber groups (md-lists/events items (if below? 'after 'before))
                                          line-of line-info))])
    (call! (if below? "open-line-below" "open-line-above") pane)
    (edit-text! pane (md-lists/prefix-edits pane (md-lists/prefixes items numbers)))))

;;; The generation the pass's last edit made. See docs/lists.md's "Renumbering after an edit".
(define md-lists/*renumbered* (box #f))

;;; See docs/lists.md's "Renumbering after an edit".
(define (md-lists/renumber-edited! pane changes)
  (let* ([generation (buffer-generation pane)]
         [edited (if (and changes
                          (not (buffer-read-only? pane))
                          (not (equal? generation (unbox md-lists/*renumbered*))))
                     (md-lists/edited-hunks changes)
                     '())])
    (unless (null? edited)
      (let* ([reader (md-preview/line-reader pane)]
             [line-of (car reader)]
             [line-info (cdr reader)]
             [groups (md-lists/groups pane (md-lists/edited-spans edited (buffer-line-count pane) line-info))]
             [edits (car (md-lists/renumber groups (md-lists/edited-events groups edited line-of line-info)
                                            line-of line-info))])
        (unless (null? edits)
          (let ([shown (buffer-panes pane)])
            (edit-text! (if (pair? shown) (car shown) pane) edits #:expect-generation generation)
            (set-box! md-lists/*renumbered* (buffer-generation pane))))))))

(define-command! "markdown/enter"
  "In a Markdown list item or quote, start the next item or quote line, renumbering an ordered list; on an empty item, end the list. Elsewhere, break the line as Enter does."
  md-lists/enter!)

(define-command! "markdown/open-below"
  "Open a line below each selection as `o` does, continuing the list item or quote it is on and renumbering an ordered list."
  (lambda (pane) (md-lists/open! pane #t))
  #:repeatable #t)

(define-command! "markdown/open-above"
  "Open a line above each selection as `O` does, continuing the list item or quote it is on and renumbering an ordered list."
  (lambda (pane) (md-lists/open! pane #f))
  #:repeatable #t)

(define (md-lists/toggle-tasks! pane)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [sels (buffer-selections pane)]
         [span-of (lambda (sel)
                    (cons (line-of (selection-start-char sel))
                          (+ (line-of (- (selection-end-char sel) 1)) 1)))]
         [spans (map span-of sels)]
         [lines (md-lists/distinct
                  (call! "stdlib/concat" (map (lambda (span) (range (car span) (cdr span))) spans)))]
         [items (md-lists/items lines (md-lists/groups pane spans) line-info)]
         [primary (span-of (call! "stdlib/primary-selection" sels))]
         [primary-items (map cdr (filter (lambda (line-item)
                                           (and (>= (car line-item) (car primary))
                                                (< (car line-item) (cdr primary))))
                                         (map cons lines items)))])
    (edit-text! pane (md-lists/task-edits items primary-items))))

(define-command! "markdown/toggle-task"
  "Check or uncheck the task boxes on the lines each selection touches: when the first box on the primary selection's lines, or else the first box of all, is checked, uncheck them all, and otherwise check them all."
  md-lists/toggle-tasks!
  #:repeatable #t)

(bind-key! 'normal "G X" "markdown/toggle-task" #:keymap md-lists/keymap)

(when md-lists/enabled
  (register-hook! 'on-text-changed
    (lambda (pane changes) (md-lists/renumber-edited! pane changes)))
  (bind-key! 'insert "enter" "markdown/enter" #:keymap md-lists/keymap)
  (bind-key! 'normal "o" "markdown/open-below" #:keymap md-lists/keymap)
  (bind-key! 'normal "O" "markdown/open-above" #:keymap md-lists/keymap))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define (md-preview/toggle! pane)
  (unless (member (buffer-key pane) (map buffer-key (buffers)))
    (error "markdown: the current buffer is not a Markdown buffer"))
  (md-preview/track! pane)
  (let ([on? (md-preview/toggle-enabled! pane)])
    (if on?
        (md-preview/render! pane)
        (md-preview/clear! pane))
    (log! 'info (if on? "markdown preview on" "markdown preview off"))))

(define md-preview/toggle-doc
  "Toggle the Markdown live preview for the current buffer, which draws Markdown without its syntax.")

(define-command! "markdown/toggle-preview" md-preview/toggle-doc md-preview/toggle!)

(define-typed-command! "toggle-markdown-preview" md-preview/toggle-doc md-preview/toggle!)

(define (md-preview/format-text! text parsed)
  (unless (string? text)
    (error "markdown: format-text/markdown takes a text and its parsed tree"))
  (md-preview/format-text text parsed md-preview/rule-width))

(define-command! "format-text/markdown"
  "Given a Markdown text and its parsed tree, the conceals, line backgrounds and highlights that draw the text without its syntax. The editor calls it for every Markdown popup and the completion documentation box."
  md-preview/format-text!)
