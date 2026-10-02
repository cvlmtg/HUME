# core:lsp

The language-server client: hover, go-to-definition (plus declaration, type-definition and
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints, and status, stop and restart of running servers.
It does not install servers. `core:lsp-install` downloads and registers them, and any
plugin that calls `register-lsp-server!` works with `core:lsp` the same way.

## Usage

```scheme
(load-plugin! "core:stdlib")

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(load-plugin! "core:lsp")
```

- **Depends on:** `core:stdlib`: diagnostics navigation, code actions and the other
  features call `stdlib/cursor-char-index`, `stdlib/primary-selection` and
  `stdlib/selection-start`/`-end` at runtime.
- **Activates on:** the first buffer with a detected language, or the first of its
  commands typed. Its `manifest.scm` declares one entry for `plugin.scm` with
  `#:languages '("*")`, every editor command below in `#:commands` and the typed commands
  in `#:typed-commands`. An explicit `#:commands`/`#:events`/`#:languages` declared before
  `load-plugin!` replaces the manifest. A
  declaration keyed only on `#:events '(on-lsp-attach)` can never activate on its own,
  since nothing is registered yet for that event to fire on; `#:languages` or a command
  gives it a real trigger.
- **User docs:** [Language Servers](https://cvlmtg.github.io/HUME/lsp.html) for the full
  walkthrough, commands, keys and settings, and
  [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-lsp) for the quick
  summary.

## Commands

| Command | Effect | Default key |
|---|---|---|
| `lsp-hover` | Show hover documentation for the symbol under the cursor | `K` |
| `lsp-goto-definition` | Go to the definition | `g d` |
| `lsp-goto-declaration` | Go to the declaration | `g D` |
| `lsp-goto-type-definition` | Go to the type definition | `g y` |
| `lsp-goto-implementation` | Go to the implementation | `g i` |
| `lsp-references` | List references in a drawer | `z r` |
| `lsp-rename` | Rename the symbol under the cursor | `G R` |
| `lsp-code-actions` | Show code actions for the cursor or selection | `z a` |
| `goto-next-diagnostic`, `goto-prev-diagnostic` | Jump to the next or previous diagnostic and show its message | `g n`, `g p` |
| `lsp-fmt` | Format the buffer or the linewise selections | none |
| `:format-source` | `lsp-fmt` from the command line | none |
| `:diagnostics` | List this buffer's diagnostics in a drawer | none |
| `:lsp-status` | Show every running server and its state, plus attached buffers' diagnostic counts | none |
| `:lsp-stop [lang]` | Stop a running server (default: the focused buffer's) | none |
| `:lsp-restart [lang]` | Stop and respawn a running server (default: the focused buffer's) | none |

## Key layout

Requests that name a destination (the four `lsp-goto-*` and diagnostic navigation) live
under `g`, beside HUME's native line gotos and structural navigation. Requests that
answer with a panel rather than a jump (the references list and the code-action menu)
live under `z`, the view prefix, the same shape `core:pickers`' finders use.

| Key | Command | Why here |
|---|---|---|
| `g d`/`g D`/`g y`/`g i` | Goto definition/declaration/type-definition/implementation | Names a destination |
| `g n`/`g p` | Next/previous diagnostic | Names a destination |
| `z r` | References | Opens a panel |
| `z a` | Code actions | Opens a panel |
| `G R` | Rename | `G` is a prefix for edit commands that sit near `g` without being gotos |
| `K` | Hover | The conventional keyword-lookup key, used often enough that a prefix would cost a keystroke |

None of these collide with a binding in HUME's default keymap, and the `z` keys beside
them belong to `core:pickers`. Every binding is a two-key sequence or a fresh top-level
key. A single-key bind over an existing prefix is a plain map insert and drops the whole
subtree under it (see `core:vim-keybind`'s README).

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | File layout, request pattern, response conventions, `lib.scm` helpers, the locations drawer, status commands |
| [`docs/features.md`](docs/features.md) | Request flags, goto and references, hover, signature help, completion, code actions, formatting, rename |
| [`docs/decorations.md`](docs/decorations.md) | Diagnostics navigation and drawer, end-of-line summary, gutter signs, inlay hints |
