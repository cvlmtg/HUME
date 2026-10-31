# core:lsp-install — Catalogs

The catalogs are generated literal sexprs in the plugin's `data/` directory, read through
`(plugin-dir)` with `read-data-file`. Each file's shape follows how it is read.

## Overview

| Catalog | File | Answers | Pin |
|---|---|---|---|
| Registrations | `registrations/<name>.scm` | What a server is registered with: command, args and config, each omitted when it is the default | Registration |
| Language lists | `language-servers.scm` | Each language's ordered servers; a server's languages are derived from it | Registration |
| Requirements | `requirements/<name>.scm` | What installing a server needs on each platform | Mason |
| Sources | `sources/<name>.scm` | How to get it: kind, version, download targets | Mason |

`language-servers.scm` is one file, because `catalog.scm` reads all of it at load. Every
other catalog is a directory holding one file per server, so a server's data is read only
when something needs that server: its `registrations/` file when the scan registers it, its
`requirements/` file when the discovery hint or a blocker check needs it, and its `sources/`
file only when it installs. A name is looked up only when it is a file in its directory
(`data-file`), so a seeded server is one with a `registrations/` file.

Registration, requirements and sources records are tagged alists (`(key . value)` or
`(key sub…)`, never a positional tuple), so one field lookup serves them all. The language
table `catalog.scm` builds is read-only: callers must not mutate what it returns.

The language catalog is also the language-to-server index. A language lists its servers in
priority order, so `:lsp-install <lang>` installs the first and the discovery hint looks
that one up.

## Generation

`registrations/` and `language-servers.scm` come from the registration pin: the Helix
commit behind `core:plum`'s treesitter.hume pin, read by `scripts/sync-grammars.py`.
`sources/` comes from the Mason pin, and `requirements/` from `sources/`, both written by
`scripts/sync-lsp-sources.py`. The two pins come from different upstreams, so their
catalogs stay apart. `scripts/README.md` has the run order.

The generator classifies every download once, so the runtime does no format or tool
guessing. A name ending in a tar suffix (`.tar.gz`, `.tgz`, `.tar.xz`, `.txz`, `.tar.bz2`)
is a tar archive, `.zip` a zip, `.gz` a single gzip file. Any other asset is a bare
executable, whose bin path equals the asset name. A download in none of these shapes is
dropped, which is why a `requirements/` row exists only for an installable one.

## `registrations/<name>.scm`

What a server is registered with.

```scheme
([(command . executable)]
 [(args arg…)]
 [(config . json-string)])
```

- A server whose fields are all defaults has the record `()`.
- `command` is present only when the server's executable differs from its name; without it
  the server runs a command named like itself. The install pipeline reads it for its
  `$PATH` note.
- `args` is present only when the server takes arguments; without it the server takes none.
- `config` is the server's config table copied as one canonical (`sort_keys`) JSON string,
  present only when the server has one.

## `language-servers.scm`

Each language's servers in priority order. Root markers are not here: they belong to the
language (`runtime/scheme/languages/<name>.scm`, or `configure-language!`'s `#:roots`).

```scheme
(lang-name (servers (server-name [(only-features feature…) | (except-features feature…)])…))
```

- A language's first server is the one `:lsp-install <lang>` installs.
- At most one filter per entry.
- A registration carries no languages. The scan turns each row into the language's default
  server list ([Registration scan](architecture.md#registration-scan)).

## `requirements/<name>.scm`

What installing a server needs on each platform. It holds only what the discovery hint and
the blocker check need, so `sources/` stays unread until something installs.

```scheme
((targets (platforms fmt tool…)…) (missing . reason))   ; installable
((blocked . kind))                                      ; stub kind, not installable
```

- `platforms` is `*` for every platform, or a list of the targets under `sources/`.
- `fmt` is `tar`, `zip`, `gz` or `raw` for a download, and `#f` for a package-manager
  install. The tools follow, in the order the check looks for them: for a download, a
  decompressor a Linux `tar` needs (`xz`, `bzip2`) or `gzip` for a `gz` row, then `curl`;
  or the one program the package manager runs (the installer runs a pypi row's tool as its
  interpreter). A tar or zip row names no extractor: the check asks `core:stdlib`'s
  `stdlib/find-extractor` for one before the tools.
- `missing` is `download` or `package`: which message a platform with no row gets.

## `sources/<name>.scm`

Install data for the server its file is named after.

- The target is one of `darwin-arm64`, `darwin-x64`, `linux-x64`, `windows-x64`. A server
  missing a target omits that row.
- Installable kinds: `github` (per-target asset, sha256 and bin path), `generic`
  (per-target file, url, sha256 and bin path), `npm` (package list and bin script), `cargo`
  (crate and bin name), `golang` (module and bin name), `pypi` (package, extras and bin
  name), `gem` (package list and bin name) and `nuget` (package and bin name).
- A package-manager row may carry `(platforms target …)` when Mason restricts it.
- Any other kind is a stub and not installable: an unsupported purl kind, a cargo crate
  pinned to a git tag rather than a crates.io version (`cargo-git`), or a source-only
  `github-build` or `generic-build` package.
- A server with no Mason equivalent has no file.

## Mason pin

`mason-pin.scm` holds one string: the `mason-org/mason-registry` release tag that
`sources/` is generated from. To move to a newer registry, change the tag and run
`python3 scripts/sync-lsp-sources.py`. The script matches Helix server names to Mason
package names through its `HELIX_TO_MASON` table and reports every Helix server left
unmatched, so a server Mason renamed is fixed in that table. It downloads every asset to
record its sha256, so a run takes a while; `scripts/README.md` describes the hash cache.
