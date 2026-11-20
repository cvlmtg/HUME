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

(register-hook! 'on-buffer-open
  (lambda (pane) (md-preview/warn-missing-grammars!)))

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
      (md-preview/clear! pane))))

;; ── Links ─────────────────────────────────────────────────────────────────────

(register-hook! 'on-paste
  (lambda (pane pasted previous) (md-links/paste-link pane pasted previous)))

(register-completion-source! "markdown:paths" md-paths/complete!
  #:target 'buffer #:priority 20 #:token-chars "/.-")

(set-completion-triggers! "markdown" "markdown:paths" (list #\( #\/))

;; ── Lists ─────────────────────────────────────────────────────────────────────

(define md-lists/keymap "markdown/lists")

(define (md-lists/groups pane lines)
  (md-preview/group
    (if (language-has-grammar? "markdown")
        (syntax-captures pane "markdown" md-lists/query
                         #:ranges (map (lambda (l) (hash 'start-line l 'end-line (+ l 1))) lines))
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
         [lines (map line-of heads)]
         [groups (md-lists/groups pane lines)]
         [items (map (lambda (item head) (and item (md-lists/past-prefix? item head) item))
                     (md-lists/items lines groups line-info)
                     heads)])
    (if (null? (filter (lambda (item) (not (and item (md-lists/empty? item)))) items))
        (edit-text! pane
                    (append (map md-lists/ended-edit items)
                            (car (md-lists/renumber groups (md-lists/removed-events items)
                                                    line-of line-info))))
        (let ([plan (md-lists/renumber groups (md-lists/events items 'after) line-of line-info)])
          (edit-text! pane (car plan))
          (insert-key! pane "enter")
          (edit-text! pane (md-lists/prefix-edits pane (md-lists/prefixes items (cdr plan))))))))

;;; See docs/lists.md's "Order of edits".
(define (md-lists/open! pane below?)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [sources (md-lists/distinct
                    (map (lambda (sel)
                           (line-of (if below? (- (selection-end-char sel) 1) (selection-start-char sel))))
                         (buffer-selections pane)))]
         [groups (md-lists/groups pane sources)]
         [items (md-lists/items sources groups line-info)]
         [plan (md-lists/renumber groups (md-lists/events items (if below? 'after 'before))
                                  line-of line-info)]
         [opened (map (lambda (line) (if below? (+ line 1) line)) sources)]
         [moves (map (lambda (edit)
                       (let* ([start (hash-ref edit 'start-char)] [line (line-of start)])
                         (list line (- start (car (line-info line))) edit)))
                     (car plan))])
    (call! (if below? "open-line-below" "open-line-above") pane)
    (edit-text! pane
                (append
                  (map (lambda (move)
                         (let* ([line (car move)]
                                [edit (list-ref move 2)]
                                [shift (length (filter (lambda (at) (<= at line)) opened))]
                                [start (+ (line->char-offset pane (+ line shift)) (list-ref move 1))])
                           (hash 'start-char start
                                 'end-char (+ start (- (hash-ref edit 'end-char) (hash-ref edit 'start-char)))
                                 'text (hash-ref edit 'text))))
                       moves)
                  (md-lists/prefix-edits pane (md-lists/prefixes items (cdr plan)))))))

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

(when md-lists/enabled
  (bind-key! 'insert "enter" "markdown/enter" #:keymap md-lists/keymap)
  (bind-key! 'normal "o" "markdown/open-below" #:keymap md-lists/keymap)
  (bind-key! 'normal "O" "markdown/open-above" #:keymap md-lists/keymap)
  (register-hook! 'on-buffer-open
    (lambda (pane) (attach-keymap! pane md-lists/keymap)))
  (register-hook! 'on-buffer-close
    (lambda (pane)
      (when (buffer-live? pane)
        (detach-keymap! pane md-lists/keymap)))))

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
