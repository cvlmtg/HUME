# HUME — LSP Server Installation (`core:lsp-install`)

Design decisions for automatic language-server download, installation, and registration.
Status: **shipped**. Kept apart from `LSP.md` (client architecture) and
`ROADMAP.md`.

## Problem

LSP support (see `LSP.md`) assumes the server binary is already on the machine and
manually registered via `register-lsp-server!` in `init.scm`. That is the single worst
onboarding step: users must find, install, and wire each server by hand. This feature
closes the gap: `:lsp-install` downloads a server, installs it under HUME's data dir, and
registers it — mirroring what the grammar pipeline already does for tree-sitter grammars.

## Placement: core:lsp-install owns the server lifecycle, core:lsp stays a client

Install, uninstall, receipts and the registration scan live in their own plugin,
`core:lsp-install`. `core:lsp` is the language-server client only and knows nothing about
servers on disk: the two meet at the editor-level registry, through the
`register-lsp-server!` / `unregister-lsp-server!` / `lsp-server-registered?`
builtins that `core:steel-server` already uses. The installer pushes registrations into
that registry; the client never asks an installer anything.

Why push rather than have `core:lsp` ask a configured installer: the registry already is
the contract, so a replacement installer (better version management, a different catalog,
another package source) is a plugin that registers servers, with nothing to configure and
no provider protocol to version. A fork of `core:lsp-install` carries its own catalogs
(read through `(plugin-dir)`), pipeline and receipt format, and `core:lsp` needs no change
for it. Swapping installers is one line in `init.scm`.

`core:plum` (the plugin manager) is not involved: it manages ordinary plugins and
grammars only. The installer has the same install/list/cleanup + scan-on-load shape
`core:plum` has. Command names keep the `lsp-` prefix (`lsp-install`, not a `plum-`
prefixed name): the command namespace is flat, and discoverability next to `core:lsp`'s
`:lsp-status`/`:lsp-stop`/`:lsp-restart` matters more than naming symmetry with
`core:plum`.

Consequence: with no `core:lsp-install` in `init.scm` there is no install, uninstall or
installed-server registration; `core:lsp` works with servers registered by hand. See
`docs/LSP.md`'s "LSP server lifecycle ownership" row for how the first placement was
arrived at.

## Architecture: two seeded data sources, two pins

Both data sources are consumed the way `grammar-sources.scm` is produced today:
a pin file names an upstream revision, a sync script (dev-time, Python) regenerates
pure-data sexpr files checked into the repo, and the runtime reads only the dumb data.
No upstream format is ever parsed inside the editor.

| Concern | Upstream | Pin | Generated data |
|---|---|---|---|
| **Registration** — which servers per language, command, args, config | `helix-editor/helix` `languages.toml` (`[[language]].language-servers` + `[language-server.*]` tables) | existing `helix-pin.scm` | `runtime/plugins/core/lsp-install/servers.scm`, `language-servers.scm`, `server-commands.scm` (and each language's root markers in `runtime/scheme/languages.scm`) |
| **Installation** — where to download, per platform | `mason-org/mason-registry` (Apache-2.0; one `package.yaml` per tool, purl sources, per-platform assets; publishes compiled `registry.json` per release tag) | new `mason-pin.scm` (registry release tag) | `runtime/plugins/core/lsp-install/sources.scm` |

One generated file per pin: a helix-pin bump touches only registration data, a mason-pin
bump only install sources — every diff traceable to one upstream. Join key = server name,
**via an explicit Helix→Mason name-mapping table** maintained in the sync script: the two
namespaces differ (Helix `pylsp` is Mason `python-lsp-server`; Helix
`vscode-json-language-server` lives in Mason's `json-lsp` package). The sync prints every
Helix server left unmatched, so drops are visible, never silent.

Sync-time (not runtime) responsibilities: filter Mason to LSP category, intersect — through
the name-mapping table — with servers the Helix data actually references (every installable
server is guaranteed wired), resolve per-platform assets, and record a sha256 per asset
(downloaded and hashed by the sync script).

### Sync scripts — one per pin

Scripts align with *pins*, not features (see `scripts/README.md`):

- **`scripts/sync-grammars.py`** (existing, extended): already fetches `languages.toml` at
  helix-pin and emits `languages.scm` + `grammar-sources.scm`; additionally emits
  `servers.scm`, `language-servers.scm` and `server-commands.scm` from the same parsed TOML. One bump, one run, all helix-derived files
  move in one diff.
- **`scripts/sync-lsp-sources.py`** (new, standalone): mason-pin → `sources.scm`.
  Standalone because it is *expensive* — it downloads every asset per server×platform to
  compute sha256s; a routine helix bump must not pay that.
- **Ordering**: the Mason script reads the checked-in `servers.scm` and `server-commands.scm` for the server-name
  intersection filter, so after a helix bump that changes server names run helix sync
  first, mason sync second. Both outputs are checked in; normally each runs alone.
- Shared sexpr-emission/pin-reading helpers move to `scripts/sync_common.py` (hyphenated
  filenames are not importable).

### Why seeded, not live-fetched

Live fetching Mason's `registry.json` was **rejected**: it moves purl parsing, asset-template
expressions, and schema-evolution risk into shipped editor code, where upstream changes break
installs on user machines; it makes `:lsp-install` irreproducible day to day; and it lets
unpinned upstream data decide which binaries get executed. Seeding moves all of that into a
dev-time script that fails loudly on the maintainer's desk, produces auditable git diffs on
every version bump, and enables deterministic offline tests. A middle ground (runtime fetch of
`registry.json` at the pinned tag) was also rejected — it keeps the runtime schema interpreter
without gaining freshness.

**Accepted tradeoff — staleness**: users get the pinned server versions, not yesterday's
release. New servers and version bumps require a pin bump + sync + commit (a one-line
maintainer action). Escape hatch: manual install + manual `register-lsp-server!`, exactly
as today.

## Seeded data format

Both files follow the `grammar-sources.scm` contract — pure data, one literal sexpr, fully
canonicalised at sync time, no defaults applied at read time — but use **tagged alists
instead of positional tuples**: install records are heterogeneous (`github` vs `npm` carry
different fields, and more kinds will come), and positional encoding does not survive
optional fields.

**Registration catalogs** (from helix-pin) — split by what registration reads, so the
scan at activation loads no field only the installer needs. `servers.scm` is keyed by
*server* and holds `args` and `config`: keying it by language would copy a multi-language
server's `config` blob once per language (typescript-language-server serves four).
`language-servers.scm` is keyed by *language* and holds the language's ordered server
list; a server's languages are the inverse of that list, so they are not stored a second
time under the server. Root markers are per-language (`roots` belongs to the
language, not the server, and languages sharing a server differ: javascript/jsx root on
`jsconfig.json`, typescript/tsx on `tsconfig.json`), so they live in `languages.scm`
(`define-language! #:roots`), not here. The scan registers each server with no languages
and writes every language's list as its default.
`server-commands.scm` holds the command of each server whose command differs from its name
(a missing server runs a command named like itself), which only the install pipeline and
`sync-lsp-sources.py` read.

```scheme
;; servers.scm
(("rust-analyzer" (args) (config))
 ("typescript-language-server"
  (args "--stdio")
  (config . "{\"hostInfo\": \"hume\", \"typescript\": {\"inlayHints\": {…}}}")))

;; language-servers.scm
(("rust" (servers ("rust-analyzer")))
 ("typescript" (servers ("typescript-language-server"))))

;; server-commands.scm
(("rust-analyzer" . "rust-analyzer")
 ("typescript-language-server" . "typescript-language-server"))
```

- **Languages live only in `language-servers.scm`.** Its language names match
  `languages.scm` (same upstream, same pin). Mason's `languages:` field uses different
  naming ("TypeScript") and would need its own mapping — dropped entirely.
- Field encoding (empty tail never `#f`, canonical JSON `config` string, delivered both ways
  by `core:lsp-install/lib/register.scm`, decoded once via `(json-parse)` at the one consuming site):
  see `runtime/plugins/core/lsp-install/README.md`'s record-shape reference. A JSON string sidesteps the fact that plain
  sexpr syntax can't tell an empty JSON array from an empty JSON object. See
  [Config delivery & per-server audit](#config-delivery--per-server-audit) for why delivering
  the same blob two ways is correct rather than a mismatch.

**`sources.scm`** (install, from mason-pin) — per-kind record shapes:

```scheme
(("rust-analyzer"
  (kind . github)
  (version . "2026-07-06")
  (repo . "rust-lang/rust-analyzer")
  (targets
   (darwin-arm64 "rust-analyzer-aarch64-apple-darwin.gz" "sha256:ab12…" "rust-analyzer")
   (darwin-x64   "rust-analyzer-x86_64-apple-darwin.gz"  "sha256:cd34…" "rust-analyzer")
   (linux-x64    "rust-analyzer-x86_64-unknown-linux-gnu.gz" "sha256:…" "rust-analyzer")
   (windows-x64  "rust-analyzer-x86_64-pc-windows-msvc.zip"  "sha256:…" "rust-analyzer.exe")))
 ("typescript-language-server"
  (kind . npm)
  (version . "5.3.0")
  (packages "typescript-language-server@5.3.0" "typescript")
  (bin . "typescript-language-server"))
 ("nls"
  (kind . cargo)
  (version . "1.17.0")
  (crate . "nickel-lang-lsp")
  (bin . "nls")))
```

- **github**: each target is `(target asset-file sha256 bin-path)` — the unpacked binary's
  path relative to the server dir. It is per-target because Mason's
  `{{source.asset.bin}}` templates resolve differently per platform (`.exe` suffix, nested
  archive dirs). The sync script resolves all templating — the runtime sees literals only.
  The asset's format decides how it is unpacked: `.tar.gz`/`.tgz`/`.tar.xz`/`.txz`/`.tar.bz2`
  with `tar -xf`, `.zip`, single-file `.gz`, or *raw* — a download with no archive
  extension is the binary itself, kept under its asset name, which the sync requires to
  equal the bin path. A target in any other format (a lone `.xz`, `.dmg`, …) is dropped at
  sync time with a report, so the runtime only ever sees a format it can unpack.
- **generic**: a Mason `pkg:generic` package with explicit download URLs. Each target is
  `(target file url sha256 bin-path)`, `file` being the local name the download is saved
  as. A package listing several files names the server's own in the sync script's
  `GENERIC_PRIMARY_FILE` (jdtls: the tarball; its `lombok.jar` is not fetched). github and
  generic share one download-verify-unpack path. A generic package that is built from
  source (`haskell-language-server`, a ghcup build) is the `generic-build` stub.
- **npm**: `packages` = main package + Mason's `extra_packages`, flattened and canonicalised
  to `name@version` strings passed straight to
  `npm install --ignore-scripts --prefix servers/<name>/`.
  `bin` = script name in `node_modules/.bin`. `version` kept separate for receipt/upgrade
  comparison.
- **cargo**: crates.io semver installs only. `crate` = the crates.io package name (may
  differ from both the server name and the bin name — `nls` is Mason/crates.io
  `nickel-lang-lsp`, installed binary `nls`), installed via
  `cargo install --locked <crate>@<version> --root servers/<name>/`. `bin` = binary name;
  the installed path is `bin/<bin>` (cargo's own `--root` layout). A Mason cargo package
  pinned to a git tag/rev instead of a crates.io version (e.g. `nil`) is not reachable via
  `cargo install crate@version` and is emitted as the `cargo-git` stub kind instead (below)
  — deferred, see `docs/ROADMAP.md`.
- **golang**: `module` (the purl's module path plus its `#subpath`, when present),
  installed via `go install -- <module>@<version>` with `GOBIN=servers/<name>/bin`.
- **pypi**: `package`, `extras` (from the purl's `?extra=` qualifier; any other qualifier
  skips the server) and `bin`. A venv is created at `servers/<name>/venv`, then
  `<venv python> -m pip install --disable-pip-version-check -- <package>[<extras>]==<version>`.
  The binary is `venv/bin/<bin>` (`venv/Scripts/<bin>.exe` on Windows).
- **gem**: `packages` (`name:version` for the main gem, then Mason's `extra_packages`),
  installed via `gem install --no-document --install-dir servers/<name> --bindir
  servers/<name>/bin`. RubyGems takes `--` as the start of native-build arguments, so the
  list follows the options directly. The binstub finds its gems through `GEM_HOME` and
  `GEM_PATH`; the receipt records them (see Installation layout).
- **nuget**: `package`, installed via `dotnet tool install <package> --tool-path
  servers/<name>/bin --version <version>`.
- **`platforms`**: an optional `(platforms target …)` on a package-manager row
  (npm, cargo, golang, pypi, gem, nuget), derived from Mason's `supported_platforms`.
  Absent means every platform. `:lsp-install` refuses a listed-elsewhere host with "not
  supported on this platform". A downloaded asset is already per-platform through its own
  targets.
- **Unsupported kinds** (`opam`, `luarocks`, `github-build`, `generic-build`, `cargo-git`, …):
  emitted as a *stub* — `(kind . opam)` plus `version`, no install fields. That is what lets
  `:lsp-install` fail naming the kind and `:lsp-catalog` mark the entry "not installable". A
  seeded server Mason doesn't carry at all (no name-mapping match) gets no entry;
  `:lsp-install` for it fails with "no install source" and `:lsp-catalog` marks it the same
  way. A Mason package that shares a seeded server's name but runs a different program
  (`cuelsp`: the seeded command is `cue lsp serve`) is listed in the sync script's
  `MASON_NOT_HELIX_SERVER` and gets no entry either.

## Config delivery & per-server audit

- **`#:settings` wired up**: pushed once as `workspace/didChangeConfiguration` after
  `initialized`, resolved per-item to answer `workspace/configuration` pull requests.
  Mechanism: `hume-lsp/src/client.rs`'s `resolve_config_section`.
- **Seeded catalog delivers its config correctly**: `servers.scm`'s `config` field is
  delivered as **both** `#:init-options` and `#:settings` by `core:lsp-install/lib/register.scm`,
  as the same blob goes to both — see
  `runtime/plugins/core/lsp-install/docs/servers.md`.
- **Per-server config audit** (all 17 seeded servers carrying a `config` blob, verified
  against each server's own source): 15 work correctly as delivered. Two —
  `actions-language-server` and `pony-lsp` — need a correction, in both cases tracing to a
  pre-existing mismatch in the upstream catalog data. `scripts/sync-grammars.py`'s
  `CONFIG_OVERRIDES` table corrects both before emission — `actions-language-server`'s blob
  is double-wrapped under its own name (the server reads `sessionToken` flat, no wrapper);
  `pony-lsp`'s blob is unwrapped (the server only reads it via a `workspace/configuration`
  pull for section `"pony-lsp"`, needing that same nesting). gopls and rust-analyzer need no
  such correction: both request `workspace/configuration` under their own name and miss the
  unwrapped blob there, but that miss is a harmless no-op for both (gopls's `Options.Set` has
  an explicit no-op `case nil:`; rust-analyzer's `apply_change_with_sink` guards on
  `json.is_null()`), since their real configuration arrives via `initializationOptions`.

## Installation layout

```
~/.local/share/hume/            ($XDG_DATA_HOME/hume; Windows %LOCALAPPDATA%\hume)
├── grammars/                   (existing)
├── plugins/                    (existing)
└── servers/
    ├── .install-lock            transient — held only during an install/uninstall
    └── rust-analyzer/
        ├── receipt.scm         written LAST — the install commit point
        └── rust-analyzer       (or node_modules/… for npm, bin/… for cargo/golang/gem/nuget, venv/… for pypi)
```

- **Per-server dir, no shared `bin/`.** Mason keeps a symlinked `bin/` dir so users can put
  it on `$PATH` for other tools; HUME is the only consumer, so the receipt's `bin-path`
  (relative to the server dir) is enough. This also sidesteps the whole Windows
  symlink/junction/shim question — junctions are directory-only and file symlinks need
  Developer Mode. One less moving part on every platform.
- **Windows spawn wrinkle**: npm's `node_modules/.bin` entries on Windows are `.cmd` shims,
  which `CreateProcess` cannot spawn directly. The client's single process-spawn site
  (`hume-lsp`'s transport) wraps `.cmd`/`.bat` commands in `cmd /C`, cfg-gated.
- **Cross-process install lock**: `:lsp-install`/`:lsp-uninstall` acquire
  `<data>/servers/.install-lock` (created with `open-output-file`, which fails on an existing file — `lock.scm`)
  before mutating `servers/`, so two HUME processes racing the same operation refuse rather
  than interleave. A lock older than an hour is treated as abandoned (the process that held
  it crashed or was killed) and replaced, with a warning. The sentinel file lives directly
  under `servers/`, excluded from the startup scan so it's never misread as an interrupted
  or orphan server install.
- **Receipt = commit point.** `receipt.scm` (pure data: name, version, bin path, and
  `env-dirs`, the environment the server needs at run time as `("KEY" . "subpath of the
  server dir")` pairs — `GEM_HOME`/`GEM_PATH` for gem-kind, empty otherwise) is written
  as the final install step. A dir without a receipt is an interrupted install: warned
  about, ignored by the scan, safely redone by `:lsp-install`. No half-installed server is
  ever registered. (This is a new mechanism, not grammar precedent — grammars have no
  receipts; they rely on delete-and-reclone idempotency.) Languages are *not* stored — the
  scan derives them from `servers.scm`, and an orphan (no seeded entry) is never
  registered anyway, so caching them would only go stale.
- **Integrity**: the installer verifies each downloaded asset against the sha256 recorded
  at sync time. GitHub release assets are not content-addressed (a tag can be re-pushed
  with different bits), unlike the grammar pipeline's pinned git SHAs — the recorded hash
  restores that property. npm-kind installs have no equivalent check: integrity rests on
  the npm registry's version immutability, and `npm install` runs with `--ignore-scripts`
  so no package lifecycle script executes during install. Accepted tradeoff. cargo-kind
  installs have no sha256 either — integrity rests on crates.io's version immutability
  plus `--locked` (pinning upstream's own published dependency graph). Unlike npm's
  `--ignore-scripts`, `cargo install` compiles the crate from source, so its `build.rs`
  build script does run — an accepted tradeoff, the same trust decision as running the
  server binary itself. golang, pypi, gem and nuget installs rest on their registries'
  version immutability the same way, and run package build or install code (`pip` may
  build an sdist; `go install` compiles). generic downloads carry a sha256 like github
  assets.

## Registration model

- **Filesystem is the SSOT for "what is installed"; seeded data is the SSOT for "how to
  run it".** When `core:lsp-install`'s `plugin.scm` activates, the scan
  reads `servers/` receipts and registers each installed server once, under its catalog
  name, then writes every catalog language's default list (per the seeded data). A directory scan is cheap. `:lsp-install` calls the same
  scan (`lsp-install/register-installed-servers!`, `register.scm`) directly right after a
  successful install — or after confirming an already-up-to-date one — so a server
  installed mid-session attaches immediately, without a restart.
  `(load-plugin! "core:lsp-install")` is the supported way to bring the plugin in. Its
  `manifest.scm` splits it into two lazy entries: `plugin.scm` (`#:languages '("*")`) registers installed servers, and `commands.scm` (`#:entry`, the
  three commands) loads the install pipeline on first use. Lazy command stubs activate their
  plugin before arity marshalling, so `:lsp-install <lang>` works before either entry has
  loaded. A trigger set keyed only on `#:events '(on-lsp-attach)` can never activate, since
  nothing is registered yet for that event to fire on; the shipped manifest uses a language
  trigger instead.
- **Last-wins registration.** `register-lsp-server!` uses *replace* semantics per name,
  matching `define-language!`. `init.scm` reads naturally: `load-plugin!` → scan
  auto-registers → a later user `register-lsp-server!` under the same name overrides. The
  scan skips a name that is already registered (`lsp-server-registered?`), which is how a
  user registration survives a rescan. A user registration under a different name serves
  a language only when that language's list names it (`set-language-servers!`). At init time replacement never races
  a running client — nothing has spawned yet. `register-lsp-server!` also works from
  command context (queued, flushed end-of-eval) — not init-only — which is what
  `:lsp-install`'s runtime registration path relies on. At runtime there are two paths:
  `:reload-config` re-registration applies on next spawn; `:lsp-install` over a server
  whose client is running (reinstall/upgrade) first shuts that client down — the same
  path `:lsp-uninstall` needs — then installs, registers, and re-attaches open buffers,
  spawning fresh.
- **Binary resolution is managed-first**: the scan registers the command as
  an *absolute path* (server dir + receipt's `bin-path`), so a managed install always spawns
  the pinned binary — no lookup-order logic in the bridge. Bare command names (from manual
  `register-lsp-server!`) resolve via `$PATH` exactly as today; a user who prefers the
  `$PATH` copy overrides with a manual registration under the server's catalog name. `:lsp-install` prints a notice when
  the command already exists on `$PATH`.
- **Orphan dirs** (installed, but no seeded entry after a pin bump renames/drops a server):
  warn at scan time, leave unregistered, suggest `:lsp-uninstall`. Never silently skipped.

## Commands and lifecycle

These are Steel commands in the `core:lsp-install` plugin — no Rust command work needed:
`:`-line string arguments already reach Steel commands (arity marshalling in
`input_stack/command.rs`), and `#:inline-output #t` displays listing output.

| Command | Behaviour |
|---|---|
| `:lsp-install [language\|server]` | A seeded server name installs that server, including one that is not the first of any language's list (ruff, tombi, …); anything else is a language, installing its first server; no argument means the current buffer's language. Downloads, verifies sha256, unpacks, writes receipt, then registers and attaches already-open buffers via the scan. Re-running against an already-up-to-date install still triggers this registration step (a no-op download, but not a no-op session effect). Tab completes the installable servers and the seeded languages. |
| `:lsp-uninstall <server>` | Unregisters the server by name, which detaches its buffers and stops each running instance (one per workspace root), then removes the server dir. |
| `:lsp-catalog` | Catalog listing (`hx --health`-style): every seeded server with languages, seeded version, installed version / not installed / update available. |

`:lsp-status` (running servers, roots, state, in-flight counts, diagnostics) is the
*runtime* view — a `core:lsp` command dispatching into Rust introspection, unchanged by
this feature (it also lists the servers `:lsp-stop` left stopped); `:lsp-catalog` is the *catalog* view. Install knowledge (receipts, seeded
lists) stays in the plugin — Rust never reads them.

- **Upgrades**: after a pin bump, `:lsp-install` compares the receipt's version against the
  seeded version and reinstalls on mismatch. No auto-upgrade.
- **Manual only** — no install-on-file-open. Discovery instead: opening a buffer whose
  language has a seeded, uninstalled server produces a one-line `:lsp-install` hint via
  the `on-language-set` hook. Only fires while `core:lsp` itself is loaded or active —
  a setup running only `core:plum` (or nothing) gets no LSP hints, matching the rest of
  the feature (see [Placement](#placement-corelsp-owns-the-server-lifecycle-end-to-end)).
  "No registered one" is checked with the `lsp-language-servers` builtin (the servers a
  language uses, empty when none is registered). Hinted at most once per language per session, and only when the
  server's install source is a supported kind and it isn't already installed — never a
  hint whose suggestion would fail or be a no-op.
- **Synchronous**: installs block the editor for their duration, exactly like grammar
  installs today, with progress reported as log lines. Async install infrastructure is
  not planned.

## v1 scope and limitations

- **Source kinds**: `pkg:github` and `pkg:generic` (prebuilt binaries — rust-analyzer,
  clangd, marksman, taplo, zls, lua-language-server, terraform-ls, jdtls, …), `pkg:npm`
  (typescript-language-server, pyright, bash-language-server, `json-lsp`/`css-lsp`
  (Mason's packages wrapping vscode-langservers-extracted), …), `pkg:cargo` restricted to
  crates.io semver versions (asm-lsp, beancount-language-server, …), `pkg:golang` (gopls),
  `pkg:pypi` (ty, pylsp, …), `pkg:gem` (ruby-lsp), and `pkg:nuget` (roslyn, fsautocomplete).
  `:lsp-install` checks the tools an install needs and fails loudly naming the missing
  one before downloading anything. Other purl kinds (`opam`, `luarocks`, `cargo-git` — a
  Mason cargo package pinned to a git tag/rev instead of a crates.io version, e.g. `nil` —
  …) fail with a loud, specific error naming the unsupported kind. jdtls installs the
  tarball only: it needs a JDK and `python3` at run time, which the installer does not
  check.
- **Several servers per language, in priority order.** A language lists *multiple*
  servers for some languages (python → `["ty", "ruff", "jedi", "pylsp", "zuban"]`,
  toml → `["taplo", "tombi"]`, go → `["gopls", "golangci-lint-lsp"]`), and their order is
  priority order. The sync keeps every listed server: `servers.scm` seeds each one that has
  a `[language-server.*]` command table (one without is dropped with a report), and
  `language-servers.scm` holds each language's ordered list. An inline-table entry's
  `only-features`/`except-features` (e.g. gjs/gts, `{ name = "typescript-language-server",
  except-features = [...] }`) is carried into the list as `(only-features "f" …)` or
  `(except-features "f" …)`. Both filters on one entry, an unknown feature name, a server
  listed twice for one language, or a listed server `servers.scm` does not seed all stop
  the sync. The feature names are the 21 in `LSP_FEATURES` in `sync-grammars.py`),
  mirrored by HUME's `LspFeature::ALL`; a test in `hume-editor` compares the two. The
  client attaches a buffer to every server of its language's list
  (`docs/LSP.md`), so installing a secondary server is useful on its own.
  `:lsp-install <lang>` installs the language's first server; `:lsp-install <name>`
  installs any seeded server by name. The scan registers each installed server once and
  sets that list, filters included, as the default list of every catalog language
  (`set-default-language-servers!`), whether or not any server in it is installed. An
  entry naming a server that is not registered serves nothing. A user's own
  `set-language-servers!` wins over that default, and may name any registered server for
  any language. The expected setup is one plugin writing defaults plus the user's
  overrides in `init.scm`; where several plugins write a language's default, the last
  write wins.
- **One server, many languages** is fully supported and cheap: one registration of
  typescript-language-server serves every language whose list names it (typescript, tsx,
  javascript, jsx), with the same config.

## Implementation shape

Each layer is a pure consumer of the one below it: a Python-only data pipeline
(`mason-pin.scm`; `sync-grammars.py` → `servers.scm` and `language-servers.scm`; `sync-lsp-sources.py` →
`sources.scm`, with the Helix→Mason name-mapping table and unmatched-server report;
shared `sync_common.py`); the generic scripting primitives below; and `core:lsp-install`
itself (Steel, a pure consumer of the previous two — scan-on-load registration,
`lsp-install`/`lsp-uninstall`/`lsp-catalog` commands, receipts, orphan
warnings, per-kind install paths, missing-server hint, user-manual + `init.scm.example`
docs — `core:plum`'s `grammars.scm` is the template. See
[Placement](#placement-corelsp-install-owns-the-server-lifecycle-corelsp-stays-a-client)).
Marshalling gotcha: the minibuffer passes the integer `1` to an arity-1 Steel command
invoked with no argument — the `lsp-install` no-arg branch must test "argument is a
string", not absence.

**Primitives the plugin relies on**: last-wins `register-lsp-server!` semantics plus a
runtime registration path (registrations are queued and flushed once per eval, not just at
init); unregister path + client shutdown (for `:lsp-uninstall` and
reinstall-while-running; by registration name, one call per server); attach already-open
buffers after registration; `lsp-server-registered?` (registry query for the scan's
manual-wins filter) and `lsp-language-servers` (registry query for the discovery hint); `(plugin-dir)` (a plugin reading its own catalogs); `run-capture!`
and `run-inline-output!` (subprocesses — the latter process-group-isolated, needed because
`#:inline-output` commands run with terminal raw mode off and Steel's `spawn-process` has
no `setpgid`; `run-capture!` is isolated the same way). Everything else is Steel's own
stdlib: `which`, `open-output-file` for the atomic lock create, `read-dir-iter` for the
symlink-safe tree walk, the time and metadata functions for lock staleness,
`current-os!`/`target-arch!` for the Mason target. No install-specific Rust remains.

### Required external tools

sha256 verification and archive unpacking shell out to each platform's
canonical system tool rather than pulling in hashing/archive crates — a deliberate
choice (see below), traded for a hard runtime dependency on these being present:

| Operation | macOS | Linux | Windows |
|---|---|---|---|
| sha256 | `shasum -a 256` (ships with the OS) | `sha256sum` (coreutils) | `certutil -hashfile … SHA256` (built in) |
| `.gz` decode | `gzip -d -f` (ships with the OS) | `gzip -d -f` (ships with the OS) | `gzip -d -f` — requires Git for Windows (or equivalent) on `PATH` |
| `.zip` extract | `unzip -o` (ships with the OS) | `unzip -o` (not always preinstalled — install the `unzip` package) | `tar -xf` (bsdtar, built into Windows 10+) |
| npm-kind installs | `node`/`npm` on `PATH` — required regardless of platform |
| cargo-kind installs | a Rust toolchain (`cargo` on `PATH`, e.g. via [rustup.rs](https://rustup.rs)) — required regardless of platform; compiles the crate from source, so the first install of a given server can take a few minutes |
| golang-kind installs | `go` on `PATH` — required regardless of platform |
| pypi-kind installs | `python3` (`python` on Windows) with the `venv` and `pip` modules |
| gem-kind installs | `gem` (`gem.cmd` on Windows) on `PATH` |
| nuget-kind installs | `dotnet` on `PATH` |
| exec bit | `chmod 755` | `chmod 755` | none |
| tar archives | `tar` everywhere. On Linux, `.tar.xz` also needs `xz` and `.tar.bz2` needs `bzip2`; macOS and Windows `tar` decompress both themselves |

`git` and `curl` are already required by the grammar pipeline; this adds `unzip` on
Linux and `gzip` on Windows as the only new hard requirements for github/npm-kind
installs. cargo-kind installs are opt-in per server and add a Rust toolchain requirement
only for those. `:lsp-install` checks
the specific tool an install needs (via Steel's `which`)
before downloading anything, so a missing tool fails loudly naming it rather than
partway through an install. `:lsp-catalog` and the discovery hint run the same check.

**Why shell out instead of adding `sha2`/`flate2`/`zip` crate dependencies**: avoids growing the dependency tree for functionality the
OS/toolchain already ships, and — since these tools are already required by any
developer's `git`/build toolchain — costs no new install step in the common case.

**Accepted tradeoff — zip-slip protection is delegated to the system tool** (modern
Info-ZIP strips `../` entries; bsdtar refuses them by default), rather than implemented in
HUME. The residual risk is bounded by the sync-time sha256 pin: unpacking runs only
after `lsp-install/verify-sha256!` (`install.scm`, over `sha256.scm`) has confirmed the
archive matches the maintainer-vetted, hash-locked asset recorded in `sources.scm` — an attacker would need
to compromise the pinned upstream release itself, not just something interposed at install
time.

**Symlink-entry handling**: on Unix every *regular file* in the extracted tree is chmod'd
`0o755`, not just the seeded `bin-path` — a server whose layout ships a wrapper script or
sibling helper binaries needs all of them executable. The tree is walked with
`read-dir-iter`, which reports a symlink as a symlink without following it: a symlink entry
the archive tool extracted (whether it's `bin-path` itself or some other tree entry) is
neither recursed into nor chmod'd, so a malicious symlink pointing outside the server dir
can't have its target's permissions mutated, and a symlinked `bin-path` fails the
post-unpack check.
