# core:lsp

Language server features: hover, go-to-definition (+ declaration / type-definition /
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints.

Requires `core:stdlib` loaded first — diagnostics navigation calls
`stdlib/cursor-char-index` via `call!`.

## Usage

`core:lsp` does nothing on its own — it composes servers you register. Add both to your
`init.scm`, plus at least one `register-lsp-server!` call:

```scheme
(load-plugin "core:stdlib")

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(declare-plugin "core:lsp"
  #:events '("on-lsp-attach")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "fmt" "lsp-code-actions" "completion-trigger"))
```

`declare-plugin` activates the first time a registered server attaches to a buffer, or the
first time one of the listed commands runs — whichever comes first. `(load-plugin "core:lsp")`
also works if you'd rather load it eagerly.

See the [user manual](../../../../user-manual/docs/lsp.md) for the full walkthrough, commands,
keys, and settings.

## Keys

| Key   | Command                       |
|-------|--------------------------------|
| `g d` | lsp-goto-definition             |
| `g D` | lsp-goto-declaration            |
| `g y` | lsp-goto-type-definition        |
| `g i` | lsp-goto-implementation         |
| `g r` | lsp-references                  |
| `g R` | lsp-rename                      |
| `g k` | lsp-hover                       |
| `g a` | lsp-code-actions                |
| `g n` | goto-next-diagnostic            |
| `g p` | goto-prev-diagnostic            |

No collisions with HUME's default `g` goto trie (`g g e h l s`) at the time these were bound —
re-check `keymap/defaults.rs` if you rebind any of the native goto leaves.

`fmt` and `diagnostics` are typed-command only (`:fmt`, `:diagnostics`) — no default key.
`completion-trigger` is already bound to `Ctrl+Space` in Insert mode by HUME itself; the
plugin only needs to `define-command!` that exact name.

## How it works

One `plugin.scm` entry `require`s a file per feature area (`hover.scm`, `goto.scm`,
`diagnostics.scm`, `rename.scm`, `format.scm`, `actions.scm`, `sighelp.scm`,
`completion.scm`, `inlay.scm`), plus a shared `lib.scm` (capability checks, error
reporting, the viewport tracker, location-drawer helper). Every feature is the same
three-line shape: send an `lsp-request`, transform the response, call a UI or store
builtin — see `docs/LSP.md` for the full architecture and `docs/lsp/step-4.md` for each
feature's design notes.
