# core:steel-server

Registers [`steel-language-server`](https://github.com/mattwparas/steel/tree/master/crates/steel-language-server),
a language server for Scheme buffers (`.ss`/`.scm`/`.sld`), including HUME's own
`init.scm` and plugin files. The server is in neither Helix's `languages.toml` nor the
Mason registry, the two sources `core:lsp-install`'s catalog is generated from, so
`:lsp-install` cannot offer it. This plugin supplies its own `cargo install` step instead.

## Usage

```scheme
(declare-plugin! "core:stdlib")
(declare-plugin! "core:lsp")
(declare-plugin! "core:steel-server")
```

- **Depends on:** nothing at load time. The plugin only calls `register-lsp-server!` and
  `lsp-registered-for-language?`, so it loads without `core:lsp`. The editor-side features
  that make the registered server useful (hover, goto, diagnostics) come from `core:lsp`,
  which in turn needs `core:stdlib`.
- **Activates on:** the first Scheme buffer, or `:steel-server-install`, whichever comes
  first. `(load-plugin! "core:steel-server")` loads it eagerly instead.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-steel-server)
  for the install walkthrough.

## Commands

| Command | Effect |
|---|---|
| `:steel-server-install` | Run `cargo install steel-language-server` and register the server for Scheme buffers |

## How it works

### Registration

At load, when `steel-language-server` is on `$PATH`, the plugin registers it for the
`scheme` language with the root marker `cog.scm`. Otherwise it logs a warning that points
at `:steel-server-install`. A `scheme` registration that already exists, such as a manual
`register-lsp-server! "scheme"` in `init.scm`, is left alone.

`:steel-server-install` logs that the server is already installed when the binary is on
`$PATH`. Otherwise it needs `cargo` on `$PATH` and fails with a pointer to rustup if not.
After `cargo install` it checks that the binary is on `$PATH` and fails with a note about
`~/.cargo/bin` if not. Then it registers the server.

### Host globals

The server has no knowledge of HUME's own Scheme builtins (`define-command!`,
`register-lsp-server!` and the rest), so it flags them as unknown identifiers while you
edit HUME config or plugin files. The plugin registers the server with `#:env` setting
`STEEL_LSP_HOME` to the `lsp-home` directory next to `plugin.scm`. That directory holds a
generated `hume-globals.scm` listing every Steel identifier HUME's layers add (builtins,
bootstrap wrappers, prelude macros and native command names), each declared with
upstream's `(#%register-global "name")` mechanism (see the
[steel-language-server README](https://github.com/mattwparas/steel/tree/master/crates/steel-language-server)).
`hume-globals.scm` is regenerated from HUME's command registry and Steel engine, as its
header comment describes, and a test fails when it drifts out of sync.

`steel-language-server` reads `STEEL_LSP_HOME` unconditionally and panics at startup if the
directory is missing. The plugin therefore resolves the directory to `#f` when the runtime
directory is unavailable or `hume-globals.scm` is absent, and omits the `STEEL_LSP_HOME`
entry in that case. It also logs a warning that HUME builtins will be flagged as unknown
identifiers.
