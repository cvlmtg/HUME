# HUME — LSP Server Installation (`core:lsp-install`)

Why `core:lsp-install` is built the way it is. How it works is in the plugin's own docs
(`runtime/plugins/core/lsp-install/README.md` and `docs/`), the sync scripts are in
`scripts/README.md`, and the user-facing behaviour is in `user-manual/docs/lsp.md`.

## Placement

Install, uninstall, receipts and the registration scan live in `core:lsp-install`.
`core:lsp` is the client only and knows nothing about servers on disk. The two meet at the
editor's server registry, through `register-lsp-server!`, `unregister-lsp-server!` and
`lsp-server-registered?`.

- **The installer pushes registrations; the client never asks an installer anything.** The
  registry already is the contract, so a replacement installer is a plugin that registers
  servers, with nothing to configure and no provider protocol to version.
- **`core:plum` is not involved.** It manages plugins and grammars only.
- **Commands keep the `lsp-` prefix.** The command namespace is flat, and sitting next to
  `:lsp-status`/`:lsp-stop`/`:lsp-restart` matters more than symmetry with `core:plum`.

## Seeded data, two pins

A pin file names an upstream revision, a dev-time sync script regenerates pure-data files
checked into the repo, and the runtime reads only those. No upstream format is parsed
inside the editor.

| Concern | Upstream | Pin |
|---|---|---|
| Registration: servers per language, command, args, config | Helix's `languages.toml`, as `cvlmtg/treesitter.hume` keeps it | `core:plum`'s `treesitter-hume-pin.scm` |
| Installation: where to download, per platform | `mason-org/mason-registry` | `mason-pin.scm` |

- **Each pin has its own generated files**, so every diff traces to one upstream.
- **The join key is the server name, through an explicit Helix→Mason table** in the sync
  script, since the two namespaces differ. The sync reports every Helix server left
  unmatched.
- **One script per pin.** The Mason sync downloads every asset to hash it; a routine Helix
  bump must not pay that.
- **Languages come from the Helix data only.** Mason's `languages:` field uses other names
  and would need a mapping of its own.
- **Records are tagged alists.** Install records differ per kind, and a positional tuple
  does not survive optional fields.
- **A server's `config` is one JSON string.** Plain sexpr syntax cannot tell an empty JSON
  array from an empty JSON object.

### Why seeded, not live-fetched

Fetching Mason's `registry.json` at runtime would move purl parsing, asset-template
expressions and schema changes into shipped editor code, where an upstream change breaks
installs on user machines. It would make `:lsp-install` give different results from day to
day, and let unpinned upstream data decide which binaries run. Seeding fails on the
maintainer's desk, shows every version bump as a git diff, and allows offline tests.
Fetching `registry.json` at the pinned tag keeps the runtime interpreter without gaining
freshness.

The cost is staleness: users get the pinned versions, and a new server or version needs a
pin bump, a sync and a commit. A manual install with a manual `register-lsp-server!`
covers anything else.

## Installation

- **Per-server directory, no shared `bin/`.** HUME is the only consumer, so the receipt's
  bin path is enough, and no platform needs symlinks, junctions or shims.
- **The receipt is the commit point.** It is written last, so a failed or stopped install
  is never current and never registered, and a failed reinstall leaves the installed
  server in place. Grammar installs use the same model.
- **Receipts store no languages.** The scan derives them from the catalog, so there is no
  copy to go stale.
- **Managed installs register an absolute path.** A managed server always spawns the
  pinned binary, with no lookup order to decide. A user who prefers the `$PATH` copy
  registers it under the same name.
- **Manual only.** No install on file open and no auto-upgrade: the editor suggests
  `:lsp-install` once per language instead.
- **Installs are synchronous**, like grammar installs.

## Integrity

- **Downloads are checked against a sha256 recorded at sync time.** A GitHub release asset
  is not content-addressed, since a tag can be pushed again with different bytes.
- **Package-manager installs have no such check.** They rest on the registry's version
  immutability. `npm install` runs with `--ignore-scripts`; `cargo install --locked` pins
  upstream's dependency graph but runs build scripts, and `pip` and `go install` may build
  too. That is the same trust decision as running the server.
- **Archive traversal is left to the system tool.** Info-ZIP strips `../` entries and
  bsdtar refuses them. Unpacking runs only after the sha256 check, so an attacker has to
  compromise the pinned release itself.

### System tools, not crates

Hashing and unpacking run each platform's own tools (`shasum`/`sha256sum`/`certutil`,
`tar`, `unzip`, `gzip`) and not `sha2`/`flate2`/`zip` crates. The tools ship with the OS
or with any developer's toolchain, so the dependency tree stays smaller at no install
step in the common case. `:lsp-install` checks the tools an install needs before it
downloads anything.

## Config delivery & per-server audit

A record's `config` is delivered as both `#:init-options` and `#:settings`: pushed once as
`workspace/didChangeConfiguration`, and the source of the answers to `workspace/configuration` pulls
(`resolve_config_section`, `hume-lsp/src/server_request.rs`).

Of the 17 seeded servers with a `config`, 15 work as delivered. `actions-language-server`
and `pony-lsp` need a correction, which `scripts/sync-grammars.py`'s `CONFIG_OVERRIDES`
applies: the first reads `sessionToken` with no wrapper, the second reads its config only
from a pull for section `"pony-lsp"`. gopls and rust-analyzer pull under their own name
and miss the unwrapped blob, which does no harm: both ignore a null answer and take their
configuration from `initializationOptions`.
