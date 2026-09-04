# core:lsp

Language server features: hover, go-to-definition (+ declaration / type-definition /
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints. Also owns the LSP server lifecycle end to
end — install, uninstall, registration, and runtime management (`servers.scm`,
`registration.scm`) — see `docs/LSP-INSTALL.md` in the repository. `core:plum` (the
plugin manager) is not involved.

## Usage

```scheme
(declare-plugin "core:stdlib")

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(declare-plugin "core:lsp")
```

Requires `core:stdlib` declared or loaded first — `core:lsp` scans installed servers via
`stdlib/list-subdirs` at its own load time, and `call!`'s lazy-miss retry inline-activates a
merely declared `core:stdlib` before that scan runs; diagnostics navigation and
`:lsp-install` call `stdlib/cursor-char-index`/`stdlib/resolve-lang-arg` via `call!` too, at
runtime (see ["Depending on another
plugin"](https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin)).

The bare `declare-plugin` above resolves `manifest.scm`, which declares the plugin with
`#:languages '("*")` (any buffer with a detected language) plus every `lsp-*` command — so it
activates on the first buffer with a language, or the first `lsp-*` command typed, whichever
comes first. An explicit `#:commands`/`#:events`/`#:languages` bypasses the manifest — a
manifest keyed only on `#:events '(on-lsp-attach)` can never activate on its own, since
nothing is registered yet for that event to fire on; `#:languages`, or the four `lsp-*`
install commands (`lsp-install`, `lsp-uninstall`, `lsp-servers`, `lsp-rescan-servers`) in
`#:commands`, give it a real trigger instead. A `register-lsp-server!` override placed
before or after the `declare-plugin` line always wins over the catalog default, since the
post-load scan reads through any registration queued earlier in the same eval and skips a
language that override already claims.

See [Language Servers](https://cvlmtg.github.io/HUME/lsp.html) for the full walkthrough,
commands, keys, and settings, and
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-lsp) for the quick
summary.

## Commands

| Command                | Effect                                                                       |
|-------------------------|-------------------------------------------------------------------------------|
| `:lsp-install [lang]`  | Download, verify, unpack, and register the server for a language (default: current buffer's language) |
| `:lsp-uninstall <name>`| Shut down and unregister a server's clients, remove it from disk (by server name, not language) |
| `:lsp-servers`         | Catalog listing: every seeded server, its languages, and install status      |
| `:lsp-rescan-servers`  | Re-scan `<data>/servers/` and register any installed server not yet registered — useful for a server installed out-of-band |
| `:lsp-status`          | Show every running server and its state, plus attached buffers' diagnostic counts |
| `:lsp-stop [lang]`     | Stop a running server (default: focused buffer's)                            |
| `:lsp-restart [lang]`  | Stop and respawn a running server (default: focused buffer's)                |

## Documentation

Design and implementation notes, for contributors reading this plugin's source:

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | File layout, key layout, response conventions, `lib.scm`'s shared helpers |
| [`docs/servers.md`](docs/servers.md) | Install pipeline, config delivery, install lock, catalog/sources, discovery hint, runtime management |
| [`docs/features.md`](docs/features.md) | Goto/references, hover, signature help, completion, code actions, formatting, rename |
| [`docs/decorations.md`](docs/decorations.md) | Diagnostics navigation, EOL summary, gutter signs, inlay hints |
