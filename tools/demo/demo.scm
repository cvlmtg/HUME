;;; demo plugin — formats a markdown pipe table

(define (demo/select-row pane row)
  (goto-location! pane (hash 'target pane 'char-offset (line->char-offset pane row)))
  (select-line pane))

(define-typed-command! "demo-table"
  "Align the selected pipe table and frame it with separator rules."
  (lambda (pane)
    (let* ((sel (call! "stdlib/primary-selection" (buffer-selections pane)))
           (top (char-offset->line pane (selection-start-char sel)))
           (bottom (char-offset->line pane (- (selection-end-char sel) 1))))
      (sift-within pane "\\|")
      (align-selections pane)
      (demo/select-row pane (+ top 1))
      (sift-within pane "\\|.*\\|")
      (sift-within pane "[^|]+")
      (replace pane #\-)
      (demo/select-row pane (+ top 1))
      (yank pane)
      (demo/select-row pane bottom)
      (collapse-selection pane)
      (paste-after pane)
      (demo/select-row pane top)
      (collapse-selection pane)
      (paste-before pane))))
