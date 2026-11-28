;;; core:markdown — plugin.scm. See docs/architecture.md.

(require "lib/state.scm")
(require "lib/render.scm")
(require "lib/text.scm")
(require "lib/links.scm")
(require "lib/paths.scm")
(require "lib/list-commands.scm")
(require "lib/headings.scm")
(require "lib/table-commands.scm")
(require "lib/styles.scm")

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

(define md-preview/keymap "markdown")

(register-hook! 'on-buffer-open
  (lambda (pane)
    (md-preview/warn-missing-grammars!)
    (attach-keymap! pane md-preview/keymap)))

(register-hook! 'on-viewport-change
  (lambda (pane range headers) (md-preview/render! pane)))

(register-hook! 'on-syntax-changed
  (lambda (pane)
    (md-preview/track! pane)
    (md-preview/note-tree! pane)
    (md-links/follow-waiting! pane)
    (md-preview/render-soon! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane)
    (md-preview/remove-buffer! pane)
    (when (buffer-live? pane)
      (md-preview/clear! pane)
      (detach-keymap! pane md-preview/keymap))))

;; ── Links ─────────────────────────────────────────────────────────────────────

(register-hook! 'on-paste
  (lambda (pane pasted previous) (md-links/paste-link pane pasted previous)))

(define-command! "markdown/follow-link"
  "Follow the link under the primary cursor: open the file or heading it names, or a web address, mail address or image in the system's app. A wiki or reference link runs lsp/goto-definition. Returns #f, doing nothing, off a link, or on a wiki or reference link with no lsp/goto-definition."
  md-links/follow!)

(bind-key! 'normal "g d" "markdown/follow-link" #:keymap md-preview/keymap)

(register-completion-source! "markdown:paths" md-paths/complete!
  #:target 'buffer #:priority 20 #:token-chars "!\"$%&'*+,-./:;=?@[]^`{|}~")

(set-completion-triggers! "markdown" "markdown:paths" (list #\( #\/ #\<))

;; ── Lists ─────────────────────────────────────────────────────────────────────

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

(define-command! "markdown/toggle-task"
  "Check or uncheck the task boxes on the lines each selection touches: when the first box on the primary selection's lines, or else the first box of all, is checked, uncheck them all, and otherwise check them all."
  md-lists/toggle-tasks!
  #:repeatable #t)

(bind-key! 'normal "G X" "markdown/toggle-task" #:keymap md-preview/keymap)

;; ── Headings ─────────────────────────────────────────────────────────────────

(define-command! "markdown/heading"
  "Toggle each line a selection touches between plain text and a level-one heading; with a count, every line becomes a heading of that level (up to 6). Blank lines are left alone."
  (lambda (pane count) (md-headings/set! pane (if (= count 0) 0 (min 6 count))))
  #:repeatable #t)

(bind-key! 'normal "G H" "markdown/heading" #:keymap md-preview/keymap)

;; ── Inline styles ─────────────────────────────────────────────────────────────

(define-command! "markdown/bold"
  "Toggle bold: when the primary selection is inside bold text, remove the `**` around every selection inside bold text, and otherwise wrap each selection in `**`."
  (md-styles/toggler "bold" "**")
  #:repeatable #t)

(define-command! "markdown/italic"
  "Toggle italic: when the primary selection is inside italic text, remove the `*` around every selection inside italic text, and otherwise wrap each selection in `*`."
  (md-styles/toggler "italic" "*")
  #:repeatable #t)

(define-command! "markdown/code"
  "Toggle inline code: when the primary selection is inside inline code, remove the backticks around every selection inside inline code, and otherwise wrap each selection in backticks."
  (md-styles/toggler "code" "`")
  #:repeatable #t)

(define-command! "markdown/strikethrough"
  "Toggle strikethrough: when the primary selection is inside struck-through text, remove the `~~` around every selection inside struck-through text, and otherwise wrap each selection in `~~`."
  (md-styles/toggler "strikethrough" "~~")
  #:repeatable #t)

(bind-key! 'normal "M B" "markdown/bold" #:keymap md-preview/keymap)
(bind-key! 'normal "M I" "markdown/italic" #:keymap md-preview/keymap)
(bind-key! 'normal "M C" "markdown/code" #:keymap md-preview/keymap)
(bind-key! 'normal "M S" "markdown/strikethrough" #:keymap md-preview/keymap)

;; ── Tables ────────────────────────────────────────────────────────────────────

(define-command! "markdown/select-cell"
  "Select the text of the table cell under each selection, trimmed of blanks; a blank cell gives a cursor in it. Selections outside a table are dropped."
  (lambda (pane) (md-tables/select-cells! pane #f)))

(define-command! "markdown/select-around-cell"
  "Select the table cell under each selection with the pipe before it and its blanks, so deleting it removes the cell from its row. When the selections hold every header and body cell with text of a column, the column's delimiter cell and the pipe before each of its empty cells are selected too. Selections outside a table are dropped."
  (lambda (pane) (md-tables/select-cells! pane #t)))

(define-command! "markdown/select-column"
  "Select the text of every cell in the table column under the primary selection, one selection per row, the header included and the delimiter row left out. A row with no cell in the column, or an empty one, is left out, and a warning gives how many were."
  md-tables/select-column!)

(bind-key! 'normal "m i |" "markdown/select-cell" #:keymap md-preview/keymap)
(bind-key! 'normal "m a |" "markdown/select-around-cell" #:keymap md-preview/keymap)
(bind-key! 'normal "m |" "markdown/select-column" #:keymap md-preview/keymap)

(define-command! "markdown/move-row-up"
  "Move the table body rows the selections touch one row up, the selections with them. Nothing moves when a selection touches a header or delimiter row, or when a row would pass the first body row."
  (lambda (pane) (md-tables/move-rows! pane #f))
  #:repeatable #t)

(define-command! "markdown/move-row-down"
  "Move the table body rows the selections touch one row down, the selections with them. Nothing moves when a selection touches a header or delimiter row, or when a row would pass the last row."
  (lambda (pane) (md-tables/move-rows! pane #t))
  #:repeatable #t)

(define-command! "markdown/move-column-left"
  "Move the table columns the selections touch one column left in every row, the selections with them. Nothing moves when a column would pass the first column, or when a row has too few cells for the move."
  (lambda (pane) (md-tables/move-columns! pane #f))
  #:repeatable #t)

(define-command! "markdown/move-column-right"
  "Move the table columns the selections touch one column right in every row, the selections with them. Nothing moves when a column would pass the last column, or when a row has too few cells for the move."
  (lambda (pane) (md-tables/move-columns! pane #t))
  #:repeatable #t)

(bind-key! 'normal "G T k" "markdown/move-row-up" #:keymap md-preview/keymap)
(bind-key! 'normal "G T up" "markdown/move-row-up" #:keymap md-preview/keymap)
(bind-key! 'normal "G T j" "markdown/move-row-down" #:keymap md-preview/keymap)
(bind-key! 'normal "G T down" "markdown/move-row-down" #:keymap md-preview/keymap)
(bind-key! 'normal "G T h" "markdown/move-column-left" #:keymap md-preview/keymap)
(bind-key! 'normal "G T left" "markdown/move-column-left" #:keymap md-preview/keymap)
(bind-key! 'normal "G T l" "markdown/move-column-right" #:keymap md-preview/keymap)
(bind-key! 'normal "G T right" "markdown/move-column-right" #:keymap md-preview/keymap)

(when md-lists/enabled
  (register-hook! 'on-text-changed
    (lambda (pane changes) (md-lists/renumber-edited! pane changes)))
  (bind-key! 'insert "enter" "markdown/enter" #:keymap md-preview/keymap)
  (bind-key! 'normal "o" "markdown/open-below" #:keymap md-preview/keymap)
  (bind-key! 'normal "O" "markdown/open-above" #:keymap md-preview/keymap))

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
