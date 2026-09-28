# core:lsp

Language server features: hover, go-to-definition (+ declaration / type-definition /
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints. Also owns the LSP server lifecycle end to end
(install, uninstall, registration, and runtime management); see `docs/LSP-INSTALL.md` in
the repository. `core:plum` (the plugin manager) is not involved.

## Usage

```scheme
(declare-plugin! "core:stdlib")

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(declare-plugin! "core:lsp")
```

- **Depends on:** `core:stdlib`: scans installed servers via `stdlib/list-subdirs` at
  its own load time; diagnostics navigation and `:lsp-install` call
  `stdlib/cursor-char-index`/`stdlib/resolve-lang-arg` at runtime.
- **Activates on:** the first buffer with a detected language, or the first `lsp-*`
  command typed. Its `manifest.scm` declares `#:languages '("*")` plus every `lsp-*`
  command. An explicit `#:commands`/`#:events`/`#:languages` bypasses it, but a manifest
  keyed only on `#:events '(on-lsp-attach)` can never activate on its own, since nothing
  is registered yet for that event to fire on; `#:languages`, or the four install
  commands (`lsp-install`, `lsp-uninstall`, `lsp-servers`, `lsp-rescan-servers`), give it
  a real trigger instead.
- **A manual `register-lsp-server!` call always wins** over the catalog default, placed
  before or after the `declare-plugin!` line, since the post-load scan reads through any
  registration queued earlier in the same eval and skips a language that override already
  claims.
- **User docs:** [Language Servers](https://cvlmtg.github.io/HUME/lsp.html) for the full
  walkthrough, commands, keys, and settings, and
  [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-lsp) for the quick
  summary.

## Commands

| Command                | Effect                                                                       |
|-------------------------|-------------------------------------------------------------------------------|
| `:lsp-install [lang]`  | Download, verify, unpack, and register the server for a language (default: current buffer's language). Tab-completes seeded languages |
| `:lsp-uninstall <name>`| Shut down and unregister a server's clients, remove it from disk (by server name, not language). Tab-completes installed servers |
| `:lsp-servers`         | Catalog listing: every seeded server, its languages, and install status      |
| `:lsp-rescan-servers`  | Re-scan `<data>/servers/` and register any installed server not yet registered; useful for a server installed out-of-band |
| `:lsp-status`          | Show every running server and its state, plus attached buffers' diagnostic counts |
| `:lsp-stop [lang]`     | Stop a running server (default: focused buffer's)                            |
| `:lsp-restart [lang]`  | Stop and respawn a running server (default: focused buffer's)                |

## Shape

Every feature file follows the same three-line shape: send an `lsp-request!`, transform
the response, call a UI or store builtin.

```
lsp-request! ──▶ transform response ──▶ UI builtin (show-popup!/show-menu!/goto-location!)
                                    └─▶ store builtin (set-signs!/set-inlay-hints!/…)
```

## Key layout

Goto-shaped requests (the four `lsp-goto-*` plus diagnostic nav) live under `g`,
alongside HUME's native line gotos and structural navigation, since each one names a
destination. Requests that answer with a panel rather than a jump (references list,
code-action menu) live under `z` instead, the view prefix, since what they do is open
something over the buffer, the same shape `core:pickers`' fuzzy finders use.

| Key | Command | Why here |
|---|---|---|
| `g d`/`g D`/`g y`/`g i` | Goto definition/declaration/type-definition/implementation | Names a destination |
| `g n`/`g p` | Next/previous diagnostic | Names a destination |
| `z r` | References | Opens a panel |
| `z a` | Code actions | Opens a panel |
| `G R` | Rename | `G` is where Vim's `g`-adjacent-but-not-goto commands live (`G L`/`G U`/`G C` case transforms); nvim's own rename default `grn` is no more a goto than those are |
| `K` | Hover | Vim's own keyword-lookup key, used often enough that a prefix would be a tax |

No collisions with HUME's native leaves (`g`'s `g e h l s`, plus the structural
`f F t T a A c C u U v V`; `G`'s `L U C`; or `z`'s `z k j`), per
`crate::editor::keymap::defaults`. `z f`/`z b`/`z m` under the same view prefix belong to
`core:pickers`. Every one of these is a two-key sequence or a fresh top-level key, never
a bare key over an existing prefix: a single-key bind is a plain map insert and would
drop the whole subtree under it (see `core:vim-keybind`'s README for the shape of that
hazard). `lsp-fmt` and `:diagnostics` have no default key; they are typed-command only.

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Response conventions, `lib.scm`'s shared helpers |
| [`docs/servers.md`](docs/servers.md) | Install pipeline, config delivery, install lock, catalog/sources, discovery hint, runtime management |
| [`docs/features.md`](docs/features.md) | Goto/references, hover, signature help, completion, code actions, formatting, rename |
| [`docs/decorations.md`](docs/decorations.md) | Diagnostics navigation, EOL summary, gutter signs, inlay hints |
