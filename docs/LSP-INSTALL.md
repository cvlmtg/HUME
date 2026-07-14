# HUME — LSP Server Installation (core:lsp `servers.scm`)

Design decisions for automatic language-server download, installation, and registration.
Status: **design complete** — decisions below are pinned; implementation is broken into
three independently-planned steps (see [Implementation steps](#implementation-steps)).
Deliberately kept out of `LSP.md` (client architecture) and `ROADMAP.md`.

## Problem

LSP support (see `LSP.md`) assumes the server binary is already on the machine and
manually registered via `register-lsp-server!` in `init.scm`. That is the single worst
onboarding step: users must find, install, and wire each server by hand. This feature
closes the gap: `:lsp-install` downloads a server, installs it under HUME's data dir, and
registers it — mirroring what the grammar pipeline already does for tree-sitter grammars.

## Placement: core:lsp owns the server lifecycle end to end

LSP server install, uninstall, and registration all live in `core:lsp`
(`servers.scm` installs/uninstalls; `registration.scm` turns an installed server into a
live `register-lsp-server!` call, on plugin load, lazy activation, or right after an
install/uninstall). `core:plum` (the plugin manager) is not involved — it manages
ordinary plugins and grammars only, via its own `plugins.scm`/`grammars.scm` modules and
shared `lib.scm` helpers, with the same install/list/cleanup + scan-on-load-registration
shape `core:lsp`'s server pipeline mirrors.

Command names keep the `lsp-` prefix (`lsp-install`, not a `plum-`-prefixed name): the
command namespace is flat, and discoverability next to `core:lsp`'s other `:lsp-status` /
`:lsp-stop` / `:lsp-restart` commands matters more than any naming symmetry with
`core:plum`.

Consequence: with no `core:lsp` in `init.scm` at all, there is no LSP
install/uninstall/registration feature — `core:plum` never touches `servers/` or the LSP
catalogs. See `docs/ROADMAP.md`'s "LSP server lifecycle ownership" row for how this
placement was arrived at.

## Architecture: two seeded data sources, two pins

Both data sources are consumed the way `grammar-sources.scm` consumes Helix data today:
a pin file names an upstream revision, a sync script (dev-time, Python) regenerates
pure-data sexpr files checked into the repo, and the runtime reads only the dumb data.
No upstream format is ever parsed inside the editor.

| Concern | Upstream | Pin | Generated data |
|---|---|---|---|
| **Registration** — which server per language, command, args, root markers | `helix-editor/helix` `languages.toml` (`[[language]].language-servers` + `[language-server.*]` tables) | existing `helix-pin.scm` | `runtime/scheme/lsp-servers.scm` |
| **Installation** — where to download, per platform | `mason-org/mason-registry` (Apache-2.0; one `package.yaml` per tool, purl sources, per-platform assets; publishes compiled `registry.json` per release tag) | new `mason-pin.scm` (registry release tag) | `runtime/scheme/lsp-sources.scm` |

One generated file per pin: a helix-pin bump touches only registration data, a mason-pin
bump only install sources — every diff traceable to one upstream. Join key = server name,
**via an explicit Helix→Mason name-mapping table** maintained in the sync script: the two
namespaces genuinely differ (Helix `pylsp` is Mason `python-lsp-server`; Helix
`vscode-json-language-server` lives in Mason's `json-lsp` package). The sync prints every
Helix server left unmatched, so drops are visible, never silent.

Sync-time (not runtime) responsibilities: filter Mason to LSP category, intersect — through
the name-mapping table — with servers the Helix data actually references (every installable
server is guaranteed wired), resolve per-platform assets, and record a sha256 per asset
(downloaded and hashed by the sync script).

### Sync scripts — one per pin

Scripts align with *pins*, not features (see `scripts/sync-readme.md`):

- **`scripts/sync-grammars.py`** (existing, extended): already fetches `languages.toml` at
  helix-pin and emits `languages.scm` + `grammar-sources.scm`; additionally emits
  `lsp-servers.scm` from the same parsed TOML. One bump, one run, all helix-derived files
  move in one diff.
- **`scripts/sync-lsp-sources.py`** (new, standalone): mason-pin → `lsp-sources.scm`.
  Standalone because it is *expensive* — it downloads every asset per server×platform to
  compute sha256s; a routine helix bump must not pay that.
- **Ordering**: the Mason script reads the checked-in `lsp-servers.scm` for the server-name
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

**`lsp-servers.scm`** (registration, from helix-pin) — keyed by *server*, with a language
list that the scan fans out into one `register-lsp-server!` call per language. Keying by
language would copy a multi-language server's `settings` blob once per language
(typescript-language-server serves four); normalized beats denormalized copies. Root
markers are the one *per-language* field: in Helix, `roots` belongs to the language, not
the server, and languages sharing a server genuinely differ (javascript/jsx root on
`jsconfig.json`, typescript/tsx on `tsconfig.json`) — so each `languages` entry is
`(name marker…)` and the scan passes that entry's markers to its registration call.

```scheme
(("rust-analyzer"
  (languages ("rust" "Cargo.toml"))
  (command . "rust-analyzer")
  (args)
  (settings))
 ("typescript-language-server"
  (languages
   ("typescript" "package.json" "tsconfig.json")
   ("tsx"        "package.json" "tsconfig.json")
   ("javascript" "package.json" "jsconfig.json")
   ("jsx"        "package.json" "jsconfig.json"))
  (command . "typescript-language-server")
  (args "--stdio")
  (settings (hostInfo . "hume") (typescript (inlayHints …)))))
```

- **Languages live only in this file.** Helix language names match `languages.scm` (same
  upstream, same pin). Mason's `languages:` field uses different naming ("TypeScript") and
  would need its own mapping — dropped entirely.
- **Language lists are disjoint across servers** — enforced at sync time (see
  [v1 scope](#v1-scope-and-limitations)): each language appears under exactly one server,
  so the scan can never produce conflicting registrations.
- Every entry uses one shape: an absent/empty value is the empty tail (`(args)`,
  `(settings)`), never `#f` — consumers read one encoding.
- `settings` / `init-options` are nested alists whose entries take one of three shapes —
  `(key . scalar)`, `(key . #(elem…))` for a JSON array (`#()` when empty), or
  `(key entry…)` for a nested object (`(key)` when empty). The `#(...)` vector form is
  what disambiguates an empty array from an empty object; both would otherwise read as
  `(key)`. The plugin converts this to a Steel hash and JSON-encodes it at the existing
  `steel_to_json` boundary. The sync script translates Helix's TOML `config.*` tables and
  JSON arrays into this shape (`scripts/sync_common.py`'s `sexpr_dumps`,
  `vector_arrays=True`).

**`lsp-sources.scm`** (install, from mason-pin) — per-kind record shapes:

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
  (bin . "typescript-language-server")))
```

- **github**: each target is `(target asset-file sha256 bin-path)` — the unpacked binary's
  path relative to the server dir (for a plain-gzip asset like rust-analyzer's `.gz`, the
  name the decompressed file gets; for `.zip`, the path inside the archive). It is
  per-target because Mason's `{{source.asset.bin}}` templates resolve differently per
  platform (`.exe` suffix, nested archive dirs). The sync script resolves all templating —
  the runtime sees literals only.
- **npm**: `packages` = main package + Mason's `extra_packages`, flattened and canonicalised
  to `name@version` strings passed straight to
  `npm install --ignore-scripts --prefix servers/<name>/`.
  `bin` = script name in `node_modules/.bin`. `version` kept separate for receipt/upgrade
  comparison.
- **Unsupported kinds** (`pkg:golang`, `pkg:pypi`, `pkg:cargo`, …): emitted as a *stub* —
  `(kind . golang)` plus `version`, no install fields. That is what lets `:lsp-install`
  fail naming the kind and `:lsp-servers` mark the entry "not installable". A Helix-primary
  server Mason doesn't carry at all (no name-mapping match) gets no entry; `:lsp-install`
  for it fails with "no install source" and `:lsp-servers` marks it the same way.

## Installation layout

```
~/.local/share/hume/            ($XDG_DATA_HOME/hume; Windows %LOCALAPPDATA%\hume)
├── grammars/                   (existing)
├── plugins/                    (existing)
└── servers/
    ├── .install-lock            transient — held only during an install/uninstall
    └── rust-analyzer/
        ├── receipt.scm         written LAST — the install commit point
        └── rust-analyzer       (or node_modules/… for npm-kind installs)
```

- **Per-server dir, no shared `bin/`.** Mason keeps a symlinked `bin/` dir so users can put
  it on `$PATH` for other tools; HUME is the only consumer, so the receipt's `bin-path`
  (relative to the server dir) is enough. This also sidesteps the whole Windows
  symlink/junction/shim question — junctions are directory-only and file symlinks need
  Developer Mode. One less moving part on every platform.
- **Windows spawn wrinkle**: npm's `node_modules/.bin` entries on Windows are `.cmd` shims,
  which `CreateProcess` cannot spawn directly. The client's single process-spawn site
  (`hume-lsp`'s transport) wraps `.cmd`/`.bat` commands in `cmd /C`, cfg-gated. The
  *installer's* own `npm install` invocation (`hume-platform::process::npm_install`) spawns
  `npm.cmd` directly via `Command::new` instead — Rust's std has applied safe `.bat`/`.cmd`
  argument escaping since 1.77.2 (CVE-2024-24576), so this avoids the double
  command-line-parsing a `cmd /C npm …` wrapper would add. A defense-in-depth allowlist
  (`[A-Za-z0-9@/._+-]`) on every package spec runs before spawn either way, cfg-independent.
- **Cross-process install lock**: `:lsp-install`/`:lsp-uninstall` acquire
  `<data>/servers/.install-lock` (O_EXCL — `acquire-install-lock!`/`release-install-lock!`)
  before mutating `servers/`, so two HUME processes racing the same operation refuse rather
  than interleave. A lock older than an hour is treated as abandoned (the process that held
  it crashed or was killed) and replaced, with a warning. The sentinel file lives directly
  under `servers/`, excluded from the startup scan so it's never misread as an interrupted
  or orphan server install.
- **Receipt = commit point.** `receipt.scm` (pure data: name, version, bin path) is written
  as the final install step. A dir without a receipt is an interrupted install: warned
  about, ignored by the scan, safely redone by `:lsp-install`. No half-installed server is
  ever registered. (This is a new mechanism, not grammar precedent — grammars have no
  receipts; they rely on delete-and-reclone idempotency.) Languages are *not* stored — the
  scan derives them from `lsp-servers.scm`, and an orphan (no seeded entry) is never
  registered anyway, so caching them would only go stale.
- **Integrity**: the installer verifies each downloaded asset against the sha256 recorded
  at sync time. GitHub release assets are not content-addressed (a tag can be re-pushed
  with different bits), unlike the grammar pipeline's pinned git SHAs — the recorded hash
  restores that property. npm-kind installs have no equivalent check: integrity rests on
  the npm registry's version immutability, and `npm install` runs with `--ignore-scripts`
  so no package lifecycle script executes during install. Accepted tradeoff.

## Registration model

- **Filesystem is the SSOT for "what is installed"; seeded data is the SSOT for "how to
  run it".** On `core:lsp` load (or lazy activation — see the caveat below), the scan
  reads `servers/` receipts and registers each installed server for every language it
  serves (per the seeded data). A directory scan is cheap. `:lsp-install` calls the same
  scan (`lsp/register-installed-servers!`, `registration.scm`) directly right after a
  successful install — or after confirming an already-up-to-date one — so a server
  installed mid-session attaches immediately, without a restart. `core:lsp` also exposes
  the rescan directly as `:lsp-rescan-servers`, for servers installed out-of-band (not
  through `:lsp-install`).
  **Caveat for a lazily-declared `core:lsp`**: a manifest keyed only on
  `#:events '("on-lsp-attach")` can never activate on its own — nothing is registered
  yet, so nothing attaches, so the event that would trigger activation never fires.
  Load `core:lsp` eagerly, declare it with `#:languages` naming the languages you rely
  on it for (activation triggered by opening a matching file), or declare it with
  `#:commands` naming `lsp-install`/`lsp-uninstall`/`lsp-servers`/`lsp-rescan-servers`
  (activation triggered by typing one of those `:` commands — Lazy command stubs
  activate their plugin before arity marshalling, so `:lsp-install <lang>` on a
  not-yet-activated `core:lsp` works with no eager `(load-plugin "core:lsp")` needed).
- **Last-wins registration.** `register-lsp-server!` changes from ignore-duplicate (today
  reported as an error) to *replace* semantics, matching `define-language!`. `init.scm`
  then reads naturally: `load-plugin` → scan auto-registers → later user
  `register-lsp-server!` calls override. At init time replacement never races a running
  client — nothing has spawned yet. At runtime there are two paths: `:reload-config`
  re-registration applies on next spawn; `:lsp-install` over a server whose client is
  running (reinstall/upgrade) first shuts that client down — the same path
  `:lsp-uninstall` needs — then installs, registers, and re-attaches open buffers,
  spawning fresh. Note `register-lsp-server!` is init-only today (registrations are
  queued and flushed once after init); `:lsp-install` needs a runtime registration path
  (step 2).
- **Binary resolution is managed-first by construction**: the scan registers the command as
  an *absolute path* (server dir + receipt's `bin-path`), so a managed install always spawns
  the pinned binary — no lookup-order logic in the bridge. Bare command names (from manual
  `register-lsp-server!`) resolve via `$PATH` exactly as today; a user who prefers the
  `$PATH` copy overrides with a manual registration. `:lsp-install` prints a notice when
  the command already exists on `$PATH`.
- **Orphan dirs** (installed, but no seeded entry after a pin bump renames/drops a server):
  warn at scan time, leave unregistered, suggest `:lsp-uninstall`. Never silently skipped.

## Commands and lifecycle

All four are Steel commands in the `core:lsp` module — no Rust command work needed:
`:`-line string arguments already reach Steel commands (arity marshalling in
`command_mode.rs`), and `#:inline-output #t` displays listing output.

| Command | Behaviour |
|---|---|
| `:lsp-install [lang]` | No arg: current buffer's language. Downloads, verifies sha256, unpacks, writes receipt, then registers and attaches already-open buffers via the same scan `:lsp-rescan-servers` runs. Re-running against an already-up-to-date install still triggers this registration step (a no-op download, but not a no-op session effect) — covers a server installed out-of-band. No argument completion in v1 — Steel commands have no argument-completion path today (possible follow-up). |
| `:lsp-uninstall <server>` | Shuts down the server's running clients — plural: one per (language, root), and a multi-language server may back several — unregisters every language it serves, removes the server dir. |
| `:lsp-servers` | Catalog listing (`hx --health`-style): every seeded server with languages, seeded version, installed version / not installed / update available. |
| `:lsp-rescan-servers` | Re-scans `servers/` receipts and registers any not yet registered — the same scan that runs at `core:lsp` load and after every `:lsp-install`/`:lsp-uninstall`, callable directly for a server installed outside `:lsp-install`. |

`:lsp-status` (running servers, roots, state, in-flight counts, diagnostics) is the
*runtime* view — a `core:lsp` command dispatching into Rust introspection, unchanged by
this feature; `:lsp-servers` is the *catalog* view. Install knowledge (receipts, seeded
lists) stays in the plugin — Rust never reads them.

- **Upgrades**: after a pin bump, `:lsp-install` compares the receipt's version against the
  seeded version and reinstalls on mismatch. No auto-upgrade.
- **Manual only** — no install-on-file-open. Discovery instead: opening a buffer whose
  language has a seeded, uninstalled server produces a one-line `:lsp-install` hint via
  the `on-language-set` hook. Only fires while `core:lsp` itself is loaded or active —
  a setup running only `core:plum` (or nothing) gets no LSP hints, matching the rest of
  the feature (see [Placement](#placement-corelsp-owns-the-server-lifecycle-end-to-end)).
  "No registered one" is checked with the `lsp-registered-for-language?` builtin —
  registration state is not otherwise visible to Steel (`lsp-server-for-buffer` reports
  *attachment*, which is ordering-dependent and can't distinguish "unregistered" from
  "still starting"). Hinted at most once per language per session, and only when the
  server's install source is a supported kind and it isn't already installed — never a
  hint whose suggestion would fail or be a no-op.
- **Synchronous**: installs block the editor for their duration, exactly like grammar
  installs today, with progress reported as log lines. Async install infrastructure is
  not planned.

## v1 scope and limitations

- **Source kinds**: `pkg:github` (prebuilt release binaries — rust-analyzer, clangd,
  marksman, taplo, zls, lua-language-server, …) and `pkg:npm` (typescript-language-server,
  pyright, bash-language-server, `json-lsp`/`css-lsp` (Mason's packages wrapping
  vscode-langservers-extracted), …). npm-kind installs require node on the machine —
  `:lsp-install` preflights and fails loudly naming the missing tool before downloading
  anything. Other purl kinds (`pkg:golang` → gopls, `pkg:pypi`, `pkg:cargo`, …) fail with a
  loud, specific error naming the unsupported kind. Expand later.
- **One server per language — no multi-server support.** Helix lists ordered *multiple*
  servers for some languages (python → `["ty", "ruff", "jedi", "pylsp"]`,
  toml → `["taplo", "tombi"]`, go → `["gopls", "golangci-lint-lsp"]`). The registry holds
  one server per language and the client is single-server-per-buffer by design (an
  `LSP.md` v1 non-goal — multi-server means merging diagnostics and routing requests per
  capability, a client milestone, not an installer one). **v1 rule, enforced at sync
  time: each language is emitted under exactly one server — the first entry in Helix's
  list** (their order is priority order). Non-primary servers for a language are not
  seeded and not installable: Helix `toml → [taplo, tombi]` means only taplo is seeded
  for toml. Parsing note for the sync: Helix `language-servers` entries are not always
  strings — some are inline tables (`{ name = "typescript-language-server",
  except-features = [...] }`, e.g. gjs/gts); the sync unwraps `name` and ignores the
  feature filters, which are meaningless to a single-server client. Consequence: no two
  seeded servers ever share a language, so scan-time registration conflicts are
  impossible by construction. Multi-server support, when the client learns it, is a sync
  script + consumer change together.
- **One server, many languages** is fully supported and cheap: an installed
  typescript-language-server registers for typescript, tsx, javascript, jsx — N entries in
  the registry, same config.

## Open questions

None — all design questions resolved.

## Implementation steps

Three steps, each landing green and committable on its own, planned independently (in
order — each is a pure consumer of the previous step's contract: step 1's generated data
files, step 2's builtin signatures).

- [x] **Step 1 — data pipeline** (Python only, zero editor changes): `mason-pin.scm`;
  extend `sync-grammars.py` to emit `lsp-servers.scm`; new `sync-lsp-sources.py` emitting
  `lsp-sources.scm` (with the Helix→Mason name-mapping table and unmatched-server report);
  shared `sync_common.py`; run both and check in the generated files. Verified by
  inspecting the generated data plus mechanical cross-checks in the sync scripts: every
  `lsp-sources.scm` server has an `lsp-servers.scm` entry, every referenced language
  exists in `languages.scm`, and every language appears under exactly one server.
- [x] **Step 2 — Rust platform primitives**: last-wins `register-lsp-server!` semantics
  plus a runtime registration path (the builtin is init-only today — registrations are
  queued and flushed once after init); unregister path + client shutdown (for
  `:lsp-uninstall` and reinstall-while-running; per-language, matching the registry's
  language keying — the plugin fans out); attach already-open buffers after
  registration; new builtins the plugin needs — `lsp-registered-for-language?` (registry
  query for the discovery hint), platform/arch identifier, sha256
  verification, download (`curl-fetch` exists — audit for reuse), unpack (plain `.gz`
  single-file gzip and `.zip` at minimum), an npm-install wrapper (there is deliberately
  no generic run-process builtin and none is added — a narrow sandboxed wrapper like the
  git/curl ones; the path sandbox must learn the `servers/` root), and `$PATH` lookup
  (for the already-on-`$PATH` notice); cfg-gated Windows `.cmd`/`.bat` spawn wrap in
  `hume-lsp`'s transport (required, not optional — npm-kind servers cannot spawn on
  Windows without it). Update `LSP.md` where the new semantics invalidate it: the
  Decisions row "reject a second `register-lsp-server!`" and the Steel API index's
  init-only marking.

  **Superseded 2026-07-13 (full-trust plugin model, see `docs/ROADMAP.md`'s plugin trust
  model decision)**: the "no generic run-process builtin" framing above no longer holds —
  `servers.scm` now runs `curl`/`git`/`npm` directly through Steel's own `steel/process`
  (`command`/`spawn-process`/`which`), with all path-sandbox checks removed (plugins are
  trusted code). `curl-fetch`/`verify-sha256!`/`npm-install!`/`exe-on-path?` were removed;
  their replacements are `run-inline-output!` (a new sandbox-free Rust builtin —
  process-group-isolated spawn, needed only because `#:inline-output` commands run with
  terminal raw mode off and Steel's `spawn-process` has no `setpgid`), `sha256-file` (hash
  only; the compare-and-delete-on-mismatch logic moved to `lsp/verify-sha256!` in
  `servers.scm`), and Steel's own `which`. `unpack-gz`/`unpack-zip` survive as sandbox-free
  utility builtins (chmod + archive-format platform logic). The tool-preflight and
  zip-slip/symlink notes below are otherwise unaffected.
- [x] **Step 3 — `servers.scm`** (Steel, pure consumer of steps 1+2): scan-on-load
  registration; `lsp-install` / `lsp-uninstall` / `lsp-servers` commands; receipts; orphan
  warnings; npm install path; missing-server hint; user-manual + `init.scm.example` docs.
  `core:plum`'s `grammars.scm` is the template. Lives in `core:lsp` — see
  [Placement](#placement-corelsp-owns-the-server-lifecycle-end-to-end).
  Marshalling gotcha: the minibuffer passes the integer
  `1` to an arity-1 Steel command invoked with no argument — the `lsp-install` no-arg
  branch must test "argument is a string", not absence.

### Required external tools

sha256 verification and archive unpacking (step 2) shell out to each platform's
canonical system tool rather than pulling in hashing/archive crates — a deliberate
choice (see below), traded for a hard runtime dependency on these being present:

| Operation | macOS | Linux | Windows |
|---|---|---|---|
| sha256 | `shasum -a 256` (ships with the OS) | `sha256sum` (coreutils) | `certutil -hashfile … SHA256` (built in) |
| `.gz` decode | `gzip -dc` (ships with the OS) | `gzip -dc` (ships with the OS) | `gzip -dc` — requires Git for Windows (or equivalent) on `PATH` |
| `.zip` extract | `unzip -o` (ships with the OS) | `unzip -o` (not always preinstalled — install the `unzip` package) | `tar -xf` (bsdtar, built into Windows 10+) |
| npm-kind installs | `node`/`npm` on `PATH` — required regardless of platform |

`git` and `curl` were already required by the grammar pipeline; this adds `unzip` on
Linux and `gzip` on Windows as the only new hard requirements. `:lsp-install` preflights
the specific tool an install needs (via Steel's `which`, post-2026-07-13 — see the Step 2
update note above) before downloading anything, so a missing tool fails loudly naming it
rather than partway through an install.

**Why shell out instead of adding `sha2`/`flate2`/`zip` crate dependencies**: keeps the
audited process-spawn surface (`hume-platform/src/process.rs`) as the only place
`std::process::Command` is used, avoids growing the dependency tree for functionality the
OS/toolchain already ships, and — since these tools are already required by any
developer's `git`/build toolchain — costs no new install step in the common case.

**Accepted tradeoff — zip-slip protection is delegated to the system tool** (modern
Info-ZIP strips `../` entries; bsdtar refuses them by default), rather than implemented in
HUME. The residual risk is bounded by the sync-time sha256 pin: `unpack-zip` runs only
after `lsp/verify-sha256!` (Scheme, `servers.scm` — wraps the sandbox-free `sha256-file`
builtin; see the Step 2 update note above) has confirmed the archive matches the
maintainer-vetted, hash-locked asset recorded in `lsp-sources.scm` — an attacker would need
to compromise the pinned upstream release itself, not just something interposed at install
time.

**Symlink-entry handling**: `unpack-zip` (Unix) chmods `0o755` every *regular file* in the
extracted tree, not just the seeded `bin-path` — a server whose layout ships a wrapper
script or sibling helper binaries needs all of them executable. Every check and chmod goes
through `symlink_metadata`, never a symlink-following stat/chmod: a symlink entry the
archive tool extracted (whether it's `bin-path` itself or some other tree entry) is neither
recursed into nor chmod'd, so a malicious symlink pointing outside the server dir can't have
its target's permissions mutated.
