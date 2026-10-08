# core:lsp-install

Downloads, verifies and registers language servers: `:lsp-install`, `:lsp-uninstall`,
`:lsp-catalog`. Registration goes through `register-lsp-server!`
and removal through `unregister-lsp-server!`, the editor registry that `core:lsp` and every
other plugin share. `core:lsp` therefore needs nothing from this plugin, and this plugin
needs nothing from `core:lsp`. A manual `register-lsp-server!` call under the same name as a
seeded server replaces what the scan registered for it, and a rescan leaves it alone. A
download or build of that server registers the installed copy again.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp-install")
```

- **Depends on:** `core:stdlib` (`stdlib/find`, `stdlib/list-subdirs`, `stdlib/write-file!`,
  `stdlib/delete-file!`, `stdlib/delete-dir!`, `stdlib/resolve-lang-arg`,
  `stdlib/safe-path-segment?`).
- **Activates on:** two entries, each on its own trigger. `plugin.scm` loads on the first
  buffer with a detected language: it scans `<data>/servers/`,
  registers every installed server and gives the discovery hint. It reads `servers.scm` and
  `language-servers.scm` at load, and `requirements.scm` when the hint or a blocker check first needs
  a server's requirements. `commands.scm` loads on the
  first `:lsp-install`, `:lsp-uninstall` or `:lsp-catalog`, or when Tab completes their
  argument, and brings the install pipeline with it. `manifest.scm` declares both: `plugin.scm` with
  `#:languages '("*")`, and `commands.scm`
  (an `#:entry`) with the three install-pipeline typed commands. `(load-plugin! "core:lsp-install")`
  makes both entries available lazily.
- **Replacing it:** a plugin that registers servers with `register-lsp-server!` and
  unregisters them with `unregister-lsp-server!` is a complete installer as far as
  `core:lsp` is concerned. Fork this directory to keep its pipeline, receipts and
  catalogs; neither `core:lsp` nor Rust needs to change.
- **User docs:** [Language Servers](https://cvlmtg.github.io/HUME/lsp.html#installing-servers).

How the pipeline works is in [`docs/servers.md`](docs/servers.md). The wider rationale (why
two pins, why receipts) is in `docs/LSP-INSTALL.md` in the repository.

## Files

| File | Owns |
|---|---|
| `lib/catalog.scm` | Reads `servers.scm` and `language-servers.scm` from this plugin's `data/` directory with `(plugin-dir)`, and `requirements.scm` on first use; field lookup; each language's ordered server list, its first (primary) server, and a server's languages |
| `lib/source-catalog.scm` | Reads `sources.scm`, which only the install pipeline needs |
| `lib/receipts.scm` | `<data>/servers/<name>/receipt.scm` paths, reading and writing |
| `lib/register.scm` | The scan that turns installed servers into registrations, and the default server list of every catalog language |
| `lib/blocker.scm` | This platform's requirements row for a server, and the check for what blocks installing it |
| `lib/install.scm` | The installers, and the per-kind choice between them |
| `lib/discovery.scm` | The discovery hint |
| `commands.scm` | `:lsp-install`, `:lsp-uninstall`, `:lsp-catalog`, completion source |
| `lib/lock.scm` | Cross-process install lock |
| `lib/sha256.scm`, `lib/unpack.scm` | Hashing, unpacking and chmod through system tools |
| `lib/platform.scm` | This platform's Mason target name, and whether it is Windows |
| `data/servers.scm`, `data/language-servers.scm`, `data/requirements.scm`, `data/sources.scm`, `data/mason-pin.scm` | Generated catalogs and the Mason pin, described below |

## Internals

- `lsp-install/install-blocker` (`blocker.scm`) returns the reason a server cannot be installed
  here, or `#f`. `lsp-install/row-blocker` (`blocker.scm`) is the same check for a row the caller
  already resolved.
- `lsp-install/target-row` (`blocker.scm`) returns the server's `requirements.scm` row for this
  platform, or `#f`.
- `lsp-install/package-installers` (`install.scm`) holds one `(kind installer env-dirs)` row per
  package manager; an installer takes `(name fields row dir)`, where `row` is the server's `target-row`.
- `lsp-install/download-row` (`install.scm`) returns this platform's download as
  `(asset url sha bin)`.
- `lsp-install/run-install!` (`install.scm`) installs a server into its directory and returns
  `(bin-rel . env-dirs)`: the binary's path inside it and the receipt's `env-dirs`.

## Catalogs

The catalogs are generated, single literal sexprs. `servers.scm` (one record per server),
and `language-servers.scm` (one record per language)
come from the registration pin (`scripts/sync-grammars.py`), `sources.scm` from the
Mason pin (`scripts/sync-lsp-sources.py`), and `requirements.scm` from `sources.scm`
(both by `scripts/sync-lsp-sources.py`). `scripts/README.md` has the run order.

**`servers.scm`**: what a server is registered with.

```scheme
(name
 [(command . executable)]
 [(args arg…)]
 [(config . json-string)])
```

- `command` is present only when the server's executable differs from its name; without it
  the server runs a command named like itself. The install pipeline reads it for its `$PATH`
  note.
- `args` is present only when the server takes arguments; without it the server takes none.
- `config` is the server's config table copied as one canonical (`sort_keys`) JSON string,
  present only when the server has one.

**`language-servers.scm`**: each language's servers in priority
order. Root markers are not here: they belong to the language (`define-language! #:roots`).

```scheme
(lang-name (servers (server-name [(only-features feature…) | (except-features feature…)])…))
```

- A language's first server is the one `:lsp-install <lang>` installs.
- At most one filter per entry.
- A registration carries no languages. The scan hands every row's list to
  `set-default-language-servers!`, whether or not any server in it is installed: an entry
  naming a server that is not registered serves nothing. A user's own `set-language-servers!`
  wins over it, and may name any registered server for any language.
- The last write to a language's default list wins, so keep one plugin writing defaults for
  a language.

**`requirements.scm`**: what installing a server needs on each platform. The runtime reads it
when the discovery hint or a blocker check first needs a row, so it holds only what those
need, and `sources.scm` stays unread until something installs.

```scheme
(name (targets (platforms fmt tool…)…) (missing . reason))   ; installable
(name (blocked . kind))                                      ; stub kind, not installable
```

- `platforms` is `*` for every platform, or a list of the targets below.
- `fmt` is `tar`, `zip`, `gz` or `raw` for a download, and `#f` for a package-manager
  install. The tools follow, in the order the check looks for them: the program that unpacks
  the download (the installer runs the first tool of a tar or zip row), then `curl`; or the
  one program the package manager runs (the installer runs a pypi row's tool as its
  interpreter).
- `missing` is `download` or `package`: which message a platform with no row gets.
- The generator classifies every download once, so the runtime does no format or tool
  guessing. `sync-lsp-sources.py` already drops downloads that are not installable, which is
  why a row exists only for an installable one.

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
- A server with no Mason equivalent has no entry.

## Mason pin

`mason-pin.scm` holds one string: the `mason-org/mason-registry` release tag that
`sources.scm` is generated from. To move to a newer registry, change the tag and run
`python3 scripts/sync-lsp-sources.py`. If a pin bump renamed or dropped servers, run
`python3 scripts/sync-grammars.py` first. The script downloads every asset to record its
sha256, so a run takes a while; `scripts/README.md` describes the hash cache.
