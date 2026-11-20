# core:lsp

<img src="/core_lsp.png" alt="core:lsp in action" />

Language server support (hover, go-to-definition, references, symbol pickers, diagnostics, rename, formatting, code actions, signature help, completions, and inlay hints), plus management of the running server processes (`:lsp-status`, `:lsp-stop`, `:lsp-restart`). Downloading the servers is `core:lsp-install`'s job.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp")
```

Requires `core:stdlib` loaded first.

See [Language Servers](../lsp.md) for setup, the full command and key tables, and settings.
