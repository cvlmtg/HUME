;;; HUME demo config, passed with --config by tools/demo/record.sh

(load-plugin! "core:stdlib")
(load-plugin! "core:pickers")
(load-plugin! "core:lsp" #:config (hash 'inlay-hints #t))
(load-plugin! "core:undotree")
(load-plugin! "core:git-diff" #:config (hash 'branch-style 'icon))

(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(set-language-servers! "rust" '("rust-analyzer"))

(declare-plugin! "./demo.scm" #:typed-commands '("demo-table"))

(configure-statusline!
  '("Position" "steel:git-branch" "FilePath" "Language" "ReadOnly" "DirtyIndicator")
  '()
  '("MacroRecording" "SearchMatches" "Diagnostics" "KittyProtocol" "Separator" "Mode"))

;; Both files the demo edits indent with spaces.
(set-option! "indent-style" "soft")
(set-option! "gutter-signs" "always:2")
