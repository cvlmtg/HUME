# core:steel-server

Registers [`steel-language-server`](https://github.com/mattwparas/steel/tree/master/crates/steel-language-server)
for Scheme buffers (`.ss`/`.scm`/`.sld`) — which includes HUME's own `init.scm` and plugin
files, so you get hover, diagnostics, and completion while editing your HUME config. Use
alongside `core:lsp`, which provides the editor-side LSP features (hover, go-to-definition,
diagnostics navigation, etc.) that make a registered server useful.

## Temporary plugin

`steel-language-server` isn't in Helix's `languages.toml` or in mason-registry yet, so
HUME's regular server catalog (synced from those two sources) can't offer it. Once either
upstream gains it, HUME's catalog sync will pick it up automatically, `:lsp-install` will
handle it like any other server, and this plugin will be retired. (`core:lsp` can install
cargo-kind *catalog* servers directly via `:lsp-install` — but only for servers already
seeded in its catalog, which `steel-language-server` isn't, so this plugin keeps its own
global `cargo install` until upstream carries it.)

## Usage

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:lsp")
(declare-plugin "core:steel-server")
```

The zero-argument `declare-plugin` above reads this plugin's own `manifest.scm`, which
activates on the first Scheme buffer or on typing `:steel-server-install`, whichever comes
first. `(load-plugin "core:steel-server")` also works if you'd rather load it eagerly.

Your own `register-lsp-server! "scheme"` call anywhere in `init.scm` always wins over this
plugin's registration — same override rule as `core:lsp`.

## Installing the server

`:steel-server-install` runs `cargo install steel-language-server` (requires `cargo` —
install Rust from [rustup.rs](https://rustup.rs) first) and registers the server
afterward — no restart needed, an already-open Scheme buffer attaches immediately.

Manual install works just as well: run `cargo install steel-language-server` yourself in a
terminal, then run `:steel-server-install` (or restart HUME) to register it. To remove it,
`cargo uninstall steel-language-server`.

## Host globals

The server has no built-in knowledge of HUME's own Scheme builtins (`define-command!`,
`register-lsp-server!`, and the rest), so left to itself it would flag every one of them as
an unknown identifier while editing HUME config or plugin files. This plugin avoids that
automatically: it registers the server with `#:env` pointing `STEEL_LSP_HOME` at this
plugin's own `lsp-home/hume-globals.scm` — a generated file listing every Steel identifier
HUME's own layers add (builtins, bootstrap wrappers, prelude macros, native command names),
each declared via upstream's `(#%register-global "name")` mechanism (see the
[steel-language-server README](https://github.com/mattwparas/steel/tree/master/crates/steel-language-server)
for the general mechanism). `hume-globals.scm` is regenerated from the real command
registry and Steel engine — see its own header comment — and a build-time test
(`hume-editor`'s `hume_globals_scm_matches_generated_host_names`) fails if it drifts.

Setting your own `STEEL_LSP_HOME` has no effect on HUME's scheme server: this plugin's
registration always overrides it. Register `"scheme"` yourself in `init.scm` (with your own
`#:env`) if you need different host-globals wiring — same override rule as everything else
in this plugin.

## Commands

| Command                  | Effect                                                          |
|---------------------------|------------------------------------------------------------------|
| `:steel-server-install`   | Install `steel-language-server` via cargo and register it for scheme buffers |
