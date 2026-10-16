;;; core:lsp/symbols.scm — document and workspace symbol pickers. See docs/features.md.

(require "helpers.scm")
(require "locations.scm")

(define lsp/symbol-kind-names
  '("file" "module" "namespace" "package" "class" "method" "property" "field"
    "constructor" "enum" "interface" "function" "variable" "constant" "string"
    "number" "boolean" "array" "object" "key" "null" "enum-member" "struct"
    "event" "operator" "type-parameter"))

(define (lsp/symbol-kind-name kind)
  (if (and (integer? kind) (>= kind 1) (<= kind 26))
      (list-ref lsp/symbol-kind-names (- kind 1))
      "symbol"))

(define (lsp/symbol-label name kind detail)
  (string-append name "  " (lsp/symbol-kind-name kind)
                 (if (and detail (not (equal? detail "")))
                     (string-append "  " detail)
                     "")))

(define (lsp/goto-symbol! location)
  (when location (goto-location! (focused-pane) location)))

(define (lsp/symbol-rows-of answers row-of)
  (filter (lambda (row) row)
          (map row-of (apply append (map json-list answers)))))

;; ── Document symbols ─────────────────────────────────────────────────────────

(define (lsp/information-row sym)
  (cons (lsp/symbol-label (json-ref sym "name") (json-ref sym "kind")
                          (json-ref-or sym #f "containerName"))
        (json-ref sym "location")))

;; A `DocumentSymbol` and its descendants, pre-order, each with its parent's name.
(define (lsp/document-symbol-rows uri sym container)
  (let ((name (json-ref sym "name"))
        (children (json-ref-or sym #f "children")))
    (cons (cons (lsp/symbol-label name (json-ref sym "kind") container)
                (lsp-location uri (json-ref sym "selectionRange")))
          (if children
              (apply append
                     (map (lambda (child) (lsp/document-symbol-rows uri child name))
                          (json-list children)))
              '()))))

(define (lsp/document-rows uri sym)
  (if (json-contains? sym "location")
      (list (lsp/information-row sym))
      (lsp/document-symbol-rows uri sym #f)))

(define (lsp/show-document-symbols! pane uri answers)
  (let ((rows (apply append
                     (map (lambda (answer)
                            (apply append
                                   (map (lambda (sym) (lsp/document-rows uri sym))
                                        (json-list answer))))
                          answers))))
    (if (null? rows)
        (log! 'info "No symbols found")
        (picker! pane rows lsp/goto-symbol!
                 #:prompt "symbols: " #:truncate 'tail
                 #:actions (call! "stdlib/buffer-actions" lsp/goto-symbol!)))))

(define-command! "lsp/document-symbols" "Pick a symbol of the current file and jump to it."
  (lambda (pane)
    (lsp/with-servers (lsp-servers pane #:feature 'document-symbols) "document symbols"
      (lambda (servers)
        (let ((params (lsp-document-params pane)))
          (if (not params)
              (log! 'info "lsp document symbols: this buffer has no file")
              (let ((uri (hash-ref (hash-ref params "textDocument") "uri")))
                (lsp-request-all! pane "textDocument/documentSymbol" params
                  (lambda (err results)
                    (if err
                        (lsp/report-error! "document symbols" err)
                        (begin
                          (lsp/report-answer-errors! "document symbols" results)
                          (lsp/show-document-symbols! pane uri (lsp/answers results)))))
                  #:require-focus #t))))))))

;; ── Workspace symbols ────────────────────────────────────────────────────────

;; One row per symbol, #f for a location without a range, which names no place
;; to jump to. The parts are decoded one location at a time because
;; `lsp-locations->display-parts` merges locations naming the same place.
(define (lsp/workspace-row sym)
  (let ((location (json-ref sym "location")))
    (and (json-contains? location "range")
         (let ((part (car (lsp-locations->display-parts (list location)))))
           (cons (lsp/symbol-label (json-ref sym "name") (json-ref sym "kind")
                                   (lsp/location-display part))
                 (hash-ref part 'location))))))

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
                (lsp/symbol-rows-of (lsp/answers results) lsp/workspace-row)))))
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
