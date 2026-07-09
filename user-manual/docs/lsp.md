# Language Servers

`core:lsp` connects HUME to language servers: hover docs, go-to-definition, references,
diagnostics, rename, formatting, code actions, signature help, completions, and inlay hints.

## Setup

Install a language server on your system, register it, and bring in `core:lsp` from your
`init.scm`.

```scheme
(load-plugin "core:stdlib")   ; core:lsp depends on it

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(declare-plugin "core:lsp"
  #:events '("on-lsp-attach")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "fmt" "lsp-code-actions" "completion-trigger"))
```

Declaring is recommended — it keeps startup fast, and `core:lsp` activates the first time a
registered server attaches to a buffer or you run one of its commands directly. If you use
LSP in every session and would rather it load from the start, swap `declare-plugin` for
`load-plugin`:

```scheme
(load-plugin "core:lsp")
```

Opening a file whose language matches a registered server spawns it automatically (once per
project root) and attaches.

## Registering a language server

`register-lsp-server!` takes:

| Argument | Meaning |
|----------|---------|
| language | A name you choose (matches HUME's own language identity for a buffer — `"rust"`, `"python"`, `"typescript"`, …) |
| `#:command` | The executable to run |
| `#:args` | Extra command-line arguments, if the server needs them |
| `#:root-markers` | Filenames that mark a project root (HUME walks up from the opened file looking for the nearest one) |
| `#:init-options` | Server-specific initialization options, as a `hash` |
| `#:settings` | Server-specific configuration, as a `hash` — sent once at startup and answered verbatim to the server's own configuration requests |

Examples for a few commonly used servers (install the server binary yourself; HUME only
spawns it):

```scheme
;; Rust — rust-analyzer
(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

;; Python — pyright
(register-lsp-server! "python" #:command "pyright-langserver" #:args '("--stdio")
                                #:root-markers '("pyproject.toml" "setup.py"))

;; TypeScript / JavaScript — typescript-language-server
(register-lsp-server! "typescript" #:command "typescript-language-server" #:args '("--stdio")
                                    #:root-markers '("package.json" "tsconfig.json"))

;; Go — gopls
(register-lsp-server! "go" #:command "gopls" #:root-markers '("go.mod"))

;; C / C++ — clangd
(register-lsp-server! "c" #:command "clangd" #:root-markers '("compile_commands.json" ".clangd"))
```

## Commands and keys

| Key   | Command                    | Effect |
|-------|------------------------------|--------|
| `g k` | `lsp-hover`                  | Show docs for the symbol under the cursor |
| `g d` | `lsp-goto-definition`        | Jump to the symbol's definition |
| `g D` | `lsp-goto-declaration`       | Jump to the symbol's declaration |
| `g y` | `lsp-goto-type-definition`   | Jump to the symbol's type's definition |
| `g i` | `lsp-goto-implementation`    | Jump to the symbol's implementation |
| `g r` | `lsp-references`             | List every reference to the symbol |
| `g R` | `lsp-rename`                 | Rename the symbol under the cursor everywhere it's used |
| `g a` | `lsp-code-actions`           | Show fixes and refactors available at the cursor |
| `g n` | `goto-next-diagnostic`       | Jump to the next error/warning after the cursor (wraps) |
| `g p` | `goto-prev-diagnostic`       | Jump to the previous error/warning before the cursor (wraps) |
| —     | `:diagnostics`               | List every diagnostic in the buffer |
| —     | `:fmt`                       | Format the buffer, or just the selected lines if the selection spans whole lines |
| `Ctrl+Space` (Insert) | `completion-trigger` | Show completions at the cursor |

Jumping to a definition, declaration, type, implementation, or reference in another file
opens that file as a buffer; use HUME's jump-back binding to return. A goto/references
result with more than one match opens a list to pick from instead of jumping directly.

Typing while a completion menu is open narrows it; `Enter` accepts the highlighted entry,
`Esc` dismisses the menu. Signature help pops up automatically as you type an argument
list for a function the server knows about, and inlay hints (see below) appear inline once
enabled.

## Settings

| Setting | Default | Effect |
|---------|---------|--------|
| `lsp.inlay-hints` | `false` | Show inferred types and parameter names inline, next to the code they describe |
| `lsp.request-timeout-ms` | `10000` | How long to wait for a server response before giving up |
| `lsp.viewport-debounce-ms` | `150` | How long to wait after scrolling settles before refreshing viewport-driven features (like inlay hints) |
| `lsp.diagnostics-severity-floor` | `hint` | The lowest diagnostic severity shown (`error`, `warning`, `info`, or `hint`) |

```scheme
(set-option! "lsp.inlay-hints" #t)
```

## Managing servers

| Command | Effect |
|---------|--------|
| `:lsp-status` | Show every running server and its state |
| `:lsp-stop [language]` | Stop a server (default: the focused buffer's) |
| `:lsp-restart [language]` | Stop and respawn a server |

Server output and protocol errors are visible in `:messages`.

## Advanced: custom requests

`lsp-request` isn't limited to the built-in commands above — any plugin can call it to reach
a server extension the built-in feature set doesn't cover. This is how you'd add a command
for rust-analyzer's `rust-analyzer/expandMacro`, which expands the macro under the cursor and
returns its generated code:

```scheme
(define-command! "rust-expand-macro" "Show the expansion of the macro under the cursor."
  (lambda ()
    (lsp-request #f "rust-analyzer/expandMacro" (lsp-position-params (current-buffer))
      (lambda (err res)
        (cond
          (err (log! 'error (string-append "expand macro: "
                                            (if (string? err) err (hash-ref err "message")))))
          ((not res) (log! 'info "Not inside a macro"))
          (else (show-popup! (hash-ref res "expansion"))))))))
```

The shape is always the same three steps: send a request built from `lsp-position-params` or
`lsp-range-params`, transform the server's response, and hand the result to a UI or store
builtin (`show-popup!`, `show-menu!`, `show-drawer-list!`, `apply-text-edits!`,
`apply-workspace-edit!`, …). `err` and `res` are never both set — check `err` first and stop
on it, the way every built-in feature does.
