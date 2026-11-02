;;; core:lsp/symbols.scm — document and workspace symbol pickers. See docs/features.md.

(require "helpers.scm")
(require "locations.scm")

(define lsp/symbol-kind-names
  (vector "file" "module" "namespace" "package" "class" "method" "property" "field"
          "constructor" "enum" "interface" "function" "variable" "constant" "string"
          "number" "boolean" "array" "object" "key" "null" "enum-member" "struct"
          "event" "operator" "type-parameter"))

(define (lsp/symbol-kind-name kind)
  (if (and (integer? kind) (>= kind 1) (<= kind (vector-length lsp/symbol-kind-names)))
      (vector-ref lsp/symbol-kind-names (- kind 1))
      "symbol"))

;; A row's payload; see docs/features.md's "Symbols".
(struct lsp/symbol-row (target name-chars kind-chars))

(define (lsp/symbol-item name kind detail target)
  (let ((kind-name (lsp/symbol-kind-name kind)))
    (cons (string-append name "  " kind-name
                         (if (and detail (not (equal? detail "")))
                             (string-append "  " detail)
                             ""))
          (lsp/symbol-row target (string-length name) (string-length kind-name)))))

(define (lsp/goto-symbol! row)
  (when row
    (let ((target (lsp/symbol-row-target row)))
      (lsp/jump-to! (if (pair? target) (lsp-location (car target) (cdr target)) target)))))

(define (lsp/symbol-span start end scope)
  (hash 'start-char start 'end-char end 'scope scope 'selected-scope "ui.text.focus"))

(define (lsp/symbol-spans row)
  (let* ((text (car row))
         (kind-start (+ (lsp/symbol-row-name-chars (cdr row)) 2))
         (kind-end (+ kind-start (lsp/symbol-row-kind-chars (cdr row))))
         (kind (lsp/symbol-span kind-start kind-end "type")))
    (if (> (string-length text) kind-end)
        (list kind (lsp/symbol-span (+ kind-end 2) (string-length text) "comment"))
        (list kind))))

(define (lsp/render-symbol-rows rows budget)
  (map (lambda (row) (hash 'text (car row) 'spans (lsp/symbol-spans row))) rows))

;; ── Document symbols ─────────────────────────────────────────────────────────

;; Conses the rows of a `DocumentSymbol` and its descendants onto `acc`, so
;; `acc` ends up in reverse pre-order. Each row carries its parent's name.
(define (lsp/push-document-symbol-rows uri sym container acc)
  (let* ((name (json-ref sym "name"))
         (children (json-ref-or sym #f "children"))
         (acc (cons (lsp/symbol-item name (json-ref sym "kind") container
                                     (cons uri (json-ref sym "selectionRange")))
                    acc)))
    (if children
        (foldl (lambda (child acc) (lsp/push-document-symbol-rows uri child name acc))
               acc
               (json-list children))
        acc)))

(define (lsp/push-document-rows uri sym acc)
  (if (json-contains? sym "location")
      (cons (lsp/symbol-item (json-ref sym "name") (json-ref sym "kind")
                             (json-ref-or sym #f "containerName")
                             (json-ref sym "location"))
            acc)
      (lsp/push-document-symbol-rows uri sym #f acc)))

(define (lsp/show-document-symbols! pane uri results)
  (let ((rows (reverse (foldl (lambda (sym acc) (lsp/push-document-rows uri sym acc))
                              '()
                              (lsp/answer-items results)))))
    (if (null? rows)
        (log! 'info "No symbols found")
        (picker! pane rows (call! "stdlib/picker-keymap" lsp/goto-symbol!)
                 #:prompt "symbols: " #:truncate 'tail #:render lsp/render-symbol-rows))))

(define-command! "lsp/document-symbols" "Pick a symbol of the current file and jump to it."
  (lambda (pane)
    (lsp/with-servers (lsp-servers pane #:feature 'document-symbols) "document symbols"
      (lambda (servers)
        (lsp/with-params (lsp-document-params pane) "document symbols"
          (lambda (params)
            (let ((uri (hash-ref (hash-ref params "textDocument") "uri")))
              (lsp-request-all! pane "textDocument/documentSymbol" params
                (lsp/on-answers "document symbols"
                  (lambda (results) (lsp/show-document-symbols! pane uri results)))
                #:require-focus #t))))))))

;; ── Workspace symbols ────────────────────────────────────────────────────────

;; One row per symbol with a range and a location HUME can jump to.
(define (lsp/workspace-rows syms)
  (let* ((placed (filter (lambda (sym) (json-contains? (json-ref sym "location") "range")) syms))
         (places (lsp-locations->places
                  (map (lambda (sym) (json-ref sym "location")) placed))))
    (filter (lambda (row) row)
            (map (lambda (sym place)
                   (and place
                        (lsp/symbol-item (json-ref sym "name") (json-ref sym "kind")
                                         (lsp/place-display place)
                                         (hash-ref place 'location))))
                 placed places))))

(define lsp/workspace-symbols-key (lsp-supersede-key))

(define (lsp/workspace-symbol-source pane)
  (lambda (token query)
    (if (equal? query "")
        (picker-replace! token '())
        (lsp-request-all! pane "workspace/symbol" (hash "query" query)
          (lambda (err results)
            (picker-replace!
             token
             (cond
               (err (lsp/report-error! "workspace symbols" err) '())
               (else
                (lsp/report-answer-errors! "workspace symbols" results)
                (lsp/workspace-rows (lsp/answer-items results))))))
          #:allow-stale #t
          #:supersede lsp/workspace-symbols-key))))

(define-command! "lsp/workspace-symbols" "Search the symbols of the workspace and jump to one."
  (lambda (pane)
    (lsp/with-servers (lsp-servers pane #:feature 'workspace-symbols) "workspace symbols"
      (lambda (servers)
        (live-picker! pane (call! "stdlib/picker-keymap" lsp/goto-symbol!)
                      #:prompt "workspace symbols: " #:truncate 'tail
                      #:render lsp/render-symbol-rows
                      #:source (lsp/workspace-symbol-source pane))))))
