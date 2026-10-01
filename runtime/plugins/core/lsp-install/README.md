# core:lsp-install

Downloads, verifies and registers language servers: `:lsp-install`, `:lsp-uninstall`,
`:lsp-servers` and `:lsp-rescan-servers`. Registration goes through `register-lsp-server!`,
the registry `core:lsp` and every other plugin share, so `core:lsp` needs nothing from this
plugin and this plugin needs nothing from `core:lsp`. A manual `register-lsp-server!` call
always wins over what the installer registers.

## Usage

```scheme
(declare-plugin! "core:stdlib")
(declare-plugin! "core:lsp-install")
```

- **Depends on:** `core:stdlib` (`stdlib/find`, `stdlib/list-subdirs`, `stdlib/write-file!`,
  `stdlib/delete-file!`, `stdlib/delete-dir!`, `stdlib/resolve-lang-arg`,
  `stdlib/safe-path-segment?`).
- **Activates on:** the first buffer with a detected language, or the first of its four
  typed commands. Its `manifest.scm` declares `#:languages '("*")` plus those commands.
- **Writing a replacement:** a plugin that registers servers with `register-lsp-server!` and
  unregisters them with `unregister-lsp-server!` is a complete installer as far as
  `core:lsp` is concerned. Fork this directory to keep its pipeline, receipts and
  catalogs; no change to `core:lsp` or to Rust is needed.
- **User docs:** [Language Servers](https://cvlmtg.github.io/HUME/lsp.html#installing-servers).

## Files

| File | Owns |
|---|---|
| `catalog.scm` | Reads `servers.scm` and `sources.scm` from this plugin's own directory (`(plugin-dir)`); field lookup; language-to-server index |
| `receipts.scm` | `<data>/servers/<name>/receipt.scm` paths, reading and writing |
| `register.scm` | The scan that turns installed servers into registrations; `lsp-rescan-servers` |
| `install.scm` | Blocker check, tool preflight, the per-kind installers |
| `commands.scm` | `:lsp-install`, `:lsp-uninstall`, `:lsp-servers`, completion sources, discovery hint |
| `lock.scm` | Cross-process install lock |
| `sha256.scm`, `unpack.scm`, `platform.scm` | Hashing, unpacking and chmod via system tools; the Mason target name |
| `servers.scm`, `sources.scm`, `mason-pin.scm` | Generated catalogs and the Mason pin; see below |

Design notes: [`docs/servers.md`](docs/servers.md). The wider rationale (why Helix and
Mason, why receipts) is in `docs/LSP-INSTALL.md` in the repository.

## Catalogs

Both generated files are single literal sexprs, one tagged alist per server. The first
comes from the Helix pin (`scripts/sync-grammars.py`), the second from the Mason pin
(`scripts/sync-lsp-sources.py`); see `scripts/README.md` for the run order.

**`servers.scm`**: registration data.

```scheme
(name
 (languages (lang-name root-marker…)…)
 (command . cmd)
 (args arg…)
 (config . json-string))
```

`args` is the empty tail `(args)`, never `#f`, when the server takes none. `config` is
Helix's `[language-server.*.config]` table, copied verbatim as a single canonical
(`sort_keys`) JSON string; the whole tail is `(config)`, never a dotted pair, when Helix
has none. `register.scm` delivers it two ways: as `initializationOptions` (what actually
configures most servers) and as `register-lsp-server!`'s `#:settings` (answers
`workspace/configuration` pulls; a miss there is expected for servers whose config isn't
nested under their own name). The loader parses the JSON once with `(json-parse)`. One
server per language: Helix's first-listed server only, since the client is
single-server-per-buffer by design.

**`sources.scm`**: install data, joined to `servers.scm` by name. The target is one of
`darwin-arm64`, `darwin-x64`, `linux-x64`, `windows-x64`; a server missing a target simply
omits that row. Installable `kind`s: `github` (per-target asset + sha256 + bin path; the
asset is a tar archive, zip, gzip, or a raw binary), `generic` (per-target file + url +
sha256 + bin path), `npm` (package list + bin script), `cargo` (crate + bin name),
`golang` (module + bin name), `pypi` (package + extras + bin name), `gem` (package list +
bin name) and `nuget` (package + bin name). A package-manager row may carry
`(platforms target …)` when Mason restricts it. Any other `kind` is a stub, not
installable: an unsupported purl kind or a source-only `github-build`/`generic-build`
package. A Helix server with no Mason equivalent gets no entry at all.
