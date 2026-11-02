# core:steel-server

Registers [`steel-language-server`](https://github.com/mattwparas/steel/tree/master/crates/steel-language-server),
a language server for Scheme buffers (`.ss`/`.scm`/`.sld`), including HUME's own
`init.scm` and plugin files. The server is in neither of the two sources
`core:lsp-install`'s catalog is generated from, so `:lsp-install` cannot offer it. This
plugin supplies its own `cargo install` step instead.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp")
(load-plugin! "core:steel-server")
```

- **Depends on:** nothing. It calls only editor functions, no plugin's commands, so it
  loads without `core:lsp`; the registered server is useful once `core:lsp` is loaded.
- **Activates on:** the first Scheme buffer, or `:steel-server-install`.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-steel-server)
  for the install walkthrough.

## Commands

| Command | Effect |
|---|---|
| `:steel-server-install` | Run `cargo install steel-language-server` and register the server for Scheme buffers |

## How it works

### Registration

At load the plugin:

- configures the `scheme` language's root marker as `cog.scm`. A root set in `init.scm` wins
  over the plugin's whichever runs first, and the plugin's is dropped without a message.
- sets the language's default list to `steel-language-server`.
- registers the server under that name when the binary is on `$PATH`, and otherwise logs a
  warning that points at `:steel-server-install`.

`:steel-server-install` installs the binary unless it is already on `$PATH`, in which case
it logs so and installs nothing. Installing needs `cargo` on `$PATH` and fails with a
pointer to rustup if not. After `cargo install`, it fails with a note about `~/.cargo/bin`
if the binary is still not on `$PATH`. Once the binary is there, the command registers the
server.

Neither registration replaces one that already exists under that name, such as a manual
`register-lsp-server! "steel-language-server" …` in `init.scm`.

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
