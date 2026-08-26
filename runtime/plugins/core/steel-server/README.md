# core:steel-server

Registers [`steel-language-server`](https://github.com/mattwparas/steel/tree/master/crates/steel-language-server)
— a language server for Scheme buffers (`.ss`/`.scm`/`.sld`), including HUME's own
`init.scm` and plugin files.

## Usage

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:lsp")
(declare-plugin "core:steel-server")
```

Requires `core:lsp` declared or loaded first — it supplies the editor-side LSP features
(hover, goto, diagnostics) that make a registered server useful; this plugin only registers
the server itself. The bare `declare-plugin` above reads this plugin's own `manifest.scm`,
activating on the first Scheme buffer or the first `:steel-server-install`, whichever comes
first — `(load-plugin "core:steel-server")` also works, loading it eagerly instead. A manual
`register-lsp-server! "scheme"` call in `init.scm` always wins over this plugin's own
registration — same override rule as `core:lsp`. See
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-steel-server) for the
install walkthrough.

## Commands

| Command | Effect |
|---|---|
| `:steel-server-install` | Install `steel-language-server` via cargo and register it for scheme buffers |

## How it works

### Why this plugin exists

`steel-language-server` isn't in Helix's `languages.toml` or in mason-registry yet, so
HUME's regular server catalog (synced from those two sources) can't offer it. Once either
upstream gains it, HUME's catalog sync will pick it up automatically, `:lsp-install` will
handle it like any other server, and this plugin will be retired. `core:lsp` can install
cargo-kind *catalog* servers directly via `:lsp-install` — but only for servers already
seeded in its catalog, which `steel-language-server` isn't, so this plugin keeps its own
`cargo install` until upstream carries it.

### Host globals

The server has no built-in knowledge of HUME's own Scheme builtins (`define-command!`,
`register-lsp-server!`, and the rest), so left to itself it would flag every one of them as
an unknown identifier while editing HUME config or plugin files. This plugin registers the
server with `#:env` pointing `STEEL_LSP_HOME` at its own `lsp-home/hume-globals.scm` — a
generated file listing every Steel identifier HUME's own layers add (builtins, bootstrap
wrappers, prelude macros, native command names), each declared via upstream's
`(#%register-global "name")` mechanism (see the
[steel-language-server README](https://github.com/mattwparas/steel/tree/master/crates/steel-language-server)
for the general mechanism). `hume-globals.scm` is regenerated from HUME's real command
registry and Steel engine — see its own header comment — and a build-time check keeps it
from drifting out of sync.
