# core:lsp

Language server features: hover, go-to-definition (+ declaration / type-definition /
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints. Also owns LSP server *registration*: loading
this plugin scans `<data>/servers/` for PLUM-installed receipts and registers each one
(`registration.scm`) — PLUM only downloads servers to disk, it never registers them (see
`docs/LSP-INSTALL.md`).

Requires `core:stdlib` loaded first — diagnostics navigation calls
`stdlib/cursor-char-index` via `call!`.

## Usage

`core:lsp` composes servers you register — either manually via `register-lsp-server!`, or
by loading `core:plum` and installing one with `:lsp-install` (PLUM notifies this plugin
to rescan after every install). Add both to your `init.scm`, plus at least one
`register-lsp-server!` call if you're not relying on PLUM:

```scheme
(load-plugin "core:stdlib")

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(declare-plugin "core:lsp"
  #:events '("on-lsp-attach")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"))
```

`declare-plugin` activates the first time a registered server attaches to a buffer, or the
first time one of the listed commands runs — whichever comes first. `(load-plugin "core:lsp")`
also works if you'd rather load it eagerly.

**Caveat when relying on PLUM-installed servers**: a manifest keyed only on
`#:events '("on-lsp-attach")` can never activate from a PLUM install alone — nothing is
registered yet, so nothing attaches, so the event never fires. Load `core:lsp` eagerly, or
add `#:languages` naming the languages you rely on PLUM for, so opening a matching file
triggers activation (and the scan) directly.

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

`lsp-fmt` and `diagnostics` are typed-command only (`:lsp-fmt`, `:diagnostics`) — no default key.
`lsp-completion-trigger` is already bound to `Ctrl+Space` in Insert mode by HUME itself; the
plugin only needs to `define-command!` that exact name.

## How it works

One `plugin.scm` entry `require`s a file per feature area (`hover.scm`, `goto.scm`,
`diagnostics.scm`, `rename.scm`, `format.scm`, `actions.scm`, `sighelp.scm`,
`completion.scm`, `inlay.scm`), plus a shared `lib.scm` (capability checks, error
reporting, the viewport tracker, location-drawer helper) and `registration.scm` (server
registration — see below). Every feature is the same three-line shape: send an
`lsp-request`, transform the response, call a UI or store builtin.

### Server registration

`registration.scm` independently reads the seeded `runtime/scheme/lsp-servers.scm`
catalog and scans `<data>/servers/` for receipts written by PLUM's install pipeline,
registering (`register-lsp-server!`) every installed server it finds — `plugin.scm` runs
this scan once at its own top level, so it happens at load or at lazy activation. It also
exposes `:lsp-rescan-servers`, a `define-command!` PLUM's `:lsp-install` calls via `call!`
right after an install (fresh or already-up-to-date) so the server attaches immediately —
the sanctioned way for one plugin to trigger behavior in another without requiring its
module directly (see `docs/ROADMAP.md` "Plugin namespace isolation").
