# core:lsp-install

Downloads, verifies and registers language servers from a catalog generated from Helix's
language data and Mason's registry. It registers what it installs with
`register-lsp-server!` and removes it with `unregister-lsp-server!`, the editor registry
`core:lsp` reads, so neither plugin needs anything from the other.

Any plugin that makes those two calls is a complete installer as far as `core:lsp` is
concerned. Fork this directory to keep its pipeline, receipts and catalogs; neither
`core:lsp` nor Rust needs to change.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp-install")
```

- **Depends on:** `core:stdlib`, at load time for the scan of installed servers and when
  its commands run.
- **Activates on:** two lazy entries. `plugin.scm` loads on the first buffer with a
  detected language (`#:languages '("*")`). `commands.scm` loads on the first
  `:lsp-install`, `:lsp-uninstall` or `:lsp-catalog`, when Tab completes their argument,
  or on a `call!` of `lsp-install/install-server`.
- **User docs:** [Language Servers](https://cvlmtg.github.io/HUME/lsp.html#installing-servers).

## Commands

| Command | Effect |
|---|---|
| `:lsp-install [name\|lang]` | Install a seeded server by name, or a language's first server. No argument means the buffer's language. A server already at the seeded version is only registered |
| `:lsp-uninstall <name>` | Unregister a server and delete its install directory. Completes every server on disk, orphans included |
| `:lsp-catalog` | Show every seeded server with its languages and its install status or blocker |
| `lsp-install/install-server` | Install `name` over its existing install. `:lsp-install` defers a reinstall to it so the running client has stopped first |

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | File layout, the contract with `core:lsp`, install folders and receipts, the registration scan, the install lock |
| [`docs/catalogs.md`](docs/catalogs.md) | The generated catalogs, their record formats, how they are generated, the Mason pin |
| [`docs/install.md`](docs/install.md) | The install pipeline, blockers, verifying and unpacking, toolchain installs, the commands, the discovery hint |

The wider rationale (why two pins, why receipts) is in `docs/LSP-INSTALL.md` in the
repository.
