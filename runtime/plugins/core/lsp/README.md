# core:lsp

The language-server client: hover, go-to-definition (plus declaration, type-definition and
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints, and status, stop and restart of running servers.
It does not install servers. `core:lsp-install` downloads and registers them, and any
plugin that calls `register-lsp-server!` works with `core:lsp` the same way.

## Usage

```scheme
(load-plugin! "core:stdlib")

(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(set-language-servers! "rust" '("rust-analyzer"))

(load-plugin! "core:lsp")
```

- **Depends on:** `core:stdlib`, at load for config validation and at runtime for most
  commands.
- **Activates on:** the first buffer with a detected language, or the first of its
  commands.
- **User docs:** [Language Servers](https://cvlmtg.github.io/HUME/lsp.html) for the full
  walkthrough, commands, keys and settings, and
  [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-lsp) for the quick
  summary.

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'inlay-hints` | boolean | `#f` | Whether a buffer shows inlay hints until `:toggle-inlay-hints` flips it |
| `'format-max-ranges` | integer ≥ 1 | `16` | Most disjoint ranges `:format-source` sends one `rangeFormatting` request each for; see [`docs/features.md`](docs/features.md#range-requests) |

## Commands

| Key | Command | Effect |
|---|---|---|
| `K` | `lsp/hover` | Show hover documentation for the symbol under the cursor |
| `g d` | `lsp/goto-definition` | Go to the definition |
| `g D` | `lsp/goto-declaration` | Go to the declaration |
| `g y` | `lsp/goto-type-definition` | Go to the type definition |
| `g i` | `lsp/goto-implementation` | Go to the implementation |
| `z r` | `lsp/references` | List references in a drawer |
| `G R` | `lsp/rename` | Rename the symbol under the cursor |
| `z a` | `lsp/code-actions` | Show code actions for the cursor or selection |
| `z s` | `lsp/document-symbols` | Pick a symbol of the current file and jump to it |
| `z S` | `lsp/workspace-symbols` | Search the symbols of the workspace as you type and jump to one |
| `g n`, `g p` | `lsp/goto-next-diagnostic`, `lsp/goto-prev-diagnostic` | Jump to the next or previous diagnostic and show its message |
| | `lsp/format-source` | Format the buffer or the linewise selections |
| | `lsp/format-document` | Format the whole buffer, or the ranges it is given, and call `done`; for `on-before-buffer-save` handlers |
| | `:format-source` | `lsp/format-source` from the command line |
| | `:diagnostics` | List this buffer's diagnostics in a drawer |
| | `:toggle-inlay-hints` | Show or hide inlay hints in the current buffer |
| | `:lsp-status` | Show every running server and its state, plus each attached buffer's servers and diagnostic counts, and the servers a stop left stopped |
| | `:lsp-stop [name]` | Stop a running server (default: every server on the focused buffer); it stays stopped until restarted, its name is registered again or the config reloads |
| | `:lsp-restart [name]` | Stop and respawn a server, or start a stopped one (default: every server on the focused buffer) |

All keys are Normal-mode bindings;
[`docs/architecture.md`](docs/architecture.md#default-keys) says why each sits where it does.

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | File layout, default keys, request pattern, response conventions, `lib/helpers.scm` helpers, the locations drawer, status commands |
| [`docs/features.md`](docs/features.md) | Request flags, goto and references, symbols, hover, signature help, completion, code actions, formatting, rename |
| [`docs/decorations.md`](docs/decorations.md) | Diagnostics navigation and drawer, end-of-line summary, gutter signs, inlay hints |
