# core:lsp-install

Downloads, verifies and registers language servers: `:lsp-install`, `:lsp-uninstall`,
`:lsp-servers` and `:lsp-rescan-servers`. Registration goes through `register-lsp-server!`
and removal through `unregister-lsp-server!`, the editor registry that `core:lsp` and every
other plugin share. `core:lsp` therefore needs nothing from this plugin, and this plugin
needs nothing from `core:lsp`. A manual `register-lsp-server!` call always wins over what
the installer registers.

## Usage

```scheme
(declare-plugin! "core:stdlib")
(declare-plugin! "core:lsp-install")
```

- **Depends on:** `core:stdlib` (`stdlib/find`, `stdlib/list-subdirs`, `stdlib/write-file!`,
  `stdlib/delete-file!`, `stdlib/delete-dir!`, `stdlib/resolve-lang-arg`,
  `stdlib/safe-path-segment?`).
- **Activates on:** the first buffer with a detected language, or the first of its four
  typed commands. Its `manifest.scm` declares `#:languages '("*")` plus those commands. On
  activation `plugin.scm` scans `<data>/servers/` and registers every installed server.
- **Replacing it:** a plugin that registers servers with `register-lsp-server!` and
  unregisters them with `unregister-lsp-server!` is a complete installer as far as
  `core:lsp` is concerned. Fork this directory to keep its pipeline, receipts and
  catalogs; neither `core:lsp` nor Rust needs to change.
- **User docs:** [Language Servers](https://cvlmtg.github.io/HUME/lsp.html#installing-servers).

How the pipeline works is in [`docs/servers.md`](docs/servers.md). The wider rationale (why
Helix and Mason, why receipts) is in `docs/LSP-INSTALL.md` in the repository.

## Files

| File | Owns |
|---|---|
| `catalog.scm` | Reads `servers.scm` and `sources.scm` from this plugin's directory with `(plugin-dir)`; field lookup; language-to-server index |
| `receipts.scm` | `<data>/servers/<name>/receipt.scm` paths, reading and writing |
| `register.scm` | The scan that turns installed servers into registrations |
| `install.scm` | Blocker check, tool preflight, the per-kind installers |
| `commands.scm` | `:lsp-install`, `:lsp-uninstall`, `:lsp-servers`, `:lsp-rescan-servers`, completion sources, discovery hint |
| `lock.scm` | Cross-process install lock |
| `sha256.scm`, `unpack.scm`, `platform.scm` | Hashing, unpacking and chmod through system tools; the Mason target name |
| `servers.scm`, `sources.scm`, `mason-pin.scm` | Generated catalogs and the Mason pin, described below |

## Catalogs

Both catalogs are generated, single literal sexprs with one tagged alist per server.
`servers.scm` comes from the Helix pin (`scripts/sync-grammars.py`) and `sources.scm` from
the Mason pin (`scripts/sync-lsp-sources.py`). `scripts/README.md` has the run order.

**`servers.scm`**: registration data.

```scheme
(name
 (languages (lang-name root-marker…)…)
 (command . cmd)
 (args arg…)
 (config . json-string))
```

- `args` is the empty tail `(args)`, never `#f`, when the server takes none.
- `config` is Helix's `[language-server.*.config]` table copied as one canonical
  (`sort_keys`) JSON string. With no config the whole tail is `(config)`, not a dotted
  pair.
- Each language names one server: Helix's first-listed, since the client attaches one
  server per buffer.

**`sources.scm`**: install data, joined to `servers.scm` by server name.

- The target is one of `darwin-arm64`, `darwin-x64`, `linux-x64`, `windows-x64`. A server
  missing a target omits that row.
- Installable kinds: `github` (per-target asset, sha256 and bin path; the asset is a tar
  archive, zip, gzip or a raw binary), `generic` (per-target file, url, sha256 and bin
  path), `npm` (package list and bin script), `cargo` (crate and bin name), `golang`
  (module and bin name), `pypi` (package, extras and bin name), `gem` (package list and
  bin name) and `nuget` (package and bin name).
- A package-manager row may carry `(platforms target …)` when Mason restricts it.
- Any other kind is a stub and not installable: an unsupported purl kind, or a
  source-only `github-build` or `generic-build` package.
- A Helix server with no Mason equivalent has no entry.

## Mason pin

`mason-pin.scm` holds one string: the `mason-org/mason-registry` release tag that
`sources.scm` is generated from. To move to a newer registry, change the tag and run
`python3 scripts/sync-lsp-sources.py`. If a Helix pin bump renamed or dropped servers, run
`python3 scripts/sync-grammars.py` first. The script downloads every asset to record its
sha256, so a run takes a while; `scripts/README.md` describes the hash cache.
