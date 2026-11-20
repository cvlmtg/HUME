# core:steel-server

Registers a language server for Scheme buffers (`.ss`/`.scm`/`.sld`), including your own `init.scm` and plugin files, so you get hover, diagnostics, and completion while editing your HUME config. It also makes `cog.scm` the project root marker for Scheme. It needs `core:lsp`.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp")
(load-plugin! "core:steel-server")
```

The server knows HUME's own commands and configuration functions, so nothing HUME provides raises an unknown-identifier warning while you edit `init.scm` or a plugin file.

`:lsp-install` can't install this server. Install it with this plugin's own command:

| Command | Effect |
|---------|--------|
| `:steel-server-install` | Install the Scheme language server and register it for Scheme buffers |

Installing requires `cargo`; install Rust from [rustup.rs](https://rustup.rs) first. See [Language Servers](../lsp.md) for the general LSP workflow.
