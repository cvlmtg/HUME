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

(define (lsp/symbol-label name kind detail)
  (string-append name "  " (lsp/symbol-kind-name kind)
                 (if (and detail (not (equal? detail "")))
                     (string-append "  " detail)
                     "")))

;; A row's payload is a `Location`, or `(uri . range)` for a `DocumentSymbol`,
;; whose `Location` is built only for the row that is accepted.
(define (lsp/goto-symbol! payload)
  (when payload
    (lsp/jump-to! (if (pair? payload) (lsp-location (car payload) (cdr payload)) payload))))

;; ── Document symbols ─────────────────────────────────────────────────────────

(define (lsp/information-row sym)
  (cons (lsp/symbol-label (json-ref sym "name") (json-ref sym "kind")
                          (json-ref-or sym #f "containerName"))
        (json-ref sym "location")))

;; Conses the rows of a `DocumentSymbol` and its descendants onto `acc`, so
;; `acc` ends up in reverse pre-order. Each row carries its parent's name.
(define (lsp/push-document-symbol-rows uri sym container acc)
  (let* ((name (json-ref sym "name"))
         (children (json-ref-or sym #f "children"))
         (acc (cons (cons (lsp/symbol-label name (json-ref sym "kind") container)
                          (cons uri (json-ref sym "selectionRange")))
                    acc)))
    (if children
        (foldl (lambda (child acc) (lsp/push-document-symbol-rows uri child name acc))
               acc
               (json-list children))
        acc)))

(define (lsp/push-document-rows uri sym acc)
  (if (json-contains? sym "location")
      (cons (lsp/information-row sym) acc)
      (lsp/push-document-symbol-rows uri sym #f acc)))

(define (lsp/show-document-symbols! pane uri results)
  (let ((rows (reverse (foldl (lambda (sym acc) (lsp/push-document-rows uri sym acc))
                              '()
                              (lsp/answer-items results)))))
    (if (null? rows)
        (log! 'info "No symbols found")
        (picker! pane rows lsp/goto-symbol!
                 #:prompt "symbols: " #:truncate 'tail
                 #:actions (call! "stdlib/buffer-actions" lsp/goto-symbol!)))))

(define-command! "lsp/document-symbols" "Pick a symbol of the current file and jump to it."
  (lambda (pane)
    (lsp/with-servers (lsp-servers pane #:feature 'document-symbols) "document symbols"
      (lambda (servers)
        (lsp/with-params (lsp-document-params pane) "document symbols"
          (lambda (params)
            (let ((uri (hash-ref (hash-ref params "textDocument") "uri")))
              (lsp-request-all! pane "textDocument/documentSymbol" params
                (lambda (err results)
                  (if err
                      (lsp/report-error! "document symbols" err)
                      (begin
                        (lsp/report-answer-errors! "document symbols" results)
                        (lsp/show-document-symbols! pane uri results))))
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
                        (cons (lsp/symbol-label (json-ref sym "name") (json-ref sym "kind")
                                                (lsp/place-display place))
                              (hash-ref place 'location))))
                 placed places))))

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
          #:supersede "lsp-workspace-symbols"))))

(define-command! "lsp/workspace-symbols" "Search the symbols of the workspace and jump to one."
  (lambda (pane)
    (lsp/with-servers (lsp-servers pane #:feature 'workspace-symbols) "workspace symbols"
      (lambda (servers)
        (live-picker! pane lsp/goto-symbol!
                      #:prompt "workspace symbols: " #:truncate 'tail
                      #:source (lsp/workspace-symbol-source pane)
                      #:actions (call! "stdlib/buffer-actions" lsp/goto-symbol!))))))
