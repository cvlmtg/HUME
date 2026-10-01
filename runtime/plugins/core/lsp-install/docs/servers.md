# core:lsp-install — Server install and registration

## Install and registration

`install.scm` downloads, verifies, and unpacks a server (`:lsp-install`), writes a
receipt as the install commit point, and calls `register.scm`'s scan
(`lsp-install/register-installed-servers!`) directly afterward so the server attaches
immediately. The scan registers through `register-lsp-server!`, the editor-level registry
`core:lsp` and every other plugin share, so `core:lsp` needs no knowledge of this plugin
and a replacement installer needs no protocol beyond that builtin. That scan is passive
(registers already-installed servers only, no subprocess, no network) and independently
reads the seeded `servers.scm` catalog and `<data>/servers/` for receipts, registering
every installed server it finds; `plugin.scm` runs it once at its own top level, so it
also happens at load or lazy activation. It's the *only* registrar for managed servers. `:lsp-rescan-servers` exposes
the same scan for a server installed outside `:lsp-install`. A server directory with no
readable receipt (pure data, `((name . "X") (version . "V") (bin . "relative/bin/path")
(env-dirs ("KEY" . "subpath")…))`) is treated as an interrupted install, logged as a warning naming it, rather than
silently skipped.

`catalog.scm`'s accessors are read-only: callers must not mutate the value it
returns, since Scheme itself enforces nothing here. Every catalog entry, in both the
servers catalog and the sources catalog, is a tagged alist tail (`(key . value)` or `(key
sub…)`, never a positional tuple), so the shared field lookup works uniformly across both
catalogs and every field shape in them.

### Install pipeline

Installing (or reinstalling) a single server always starts from a clean slate. This also
serves as the repair/upgrade path, and covers reinstalling over a running client the same
way:

1. Blocker check + tool preflight.
2. Unregister every seeded language, reaping any running client.
3. Purge any existing install (the receipt dies with it).
4. Download, verify, and unpack (github, generic), or run the kind's package manager
   (`npm`, `cargo`, `go`, `pip` in a venv, `gem`, `dotnet tool`).
5. Write the receipt (the commit point).
6. A `$PATH` notice, if the seeded command also happens to resolve there independently of
   the managed install.

A receipt's `env-dirs` values are relative to the server dir; registration joins them with
it and passes the result as `register-lsp-server!`'s `#:env`. Only gem-kind installs write
any, since their binstub resolves gems through `GEM_HOME` and `GEM_PATH`.

One function resolves a download source for this platform (url, file, sha256, bin) and
serves the installability check, the tool preflight, and the install itself, for github
and generic alike. One table maps each toolchain kind to the tool it needs on `$PATH`,
shared by the blocker and the preflight. One check after a toolchain install confirms the
expected binary exists, shared by cargo, go, pip, gem and dotnet.

The github path's own download step recreates the install directory right after step 3
purges it: `curl -o` needs the parent directory to already exist, and nothing else
recreates it between the purge and the download.

`:lsp-install`'s own completion source lists every language a server is seeded for (the
same set its own "no language server is seeded for" check reads against); `:lsp-uninstall`'s
lists every server with an install directory on disk, seeded or orphan alike, since its
own on-disk check (not the seeded catalog) decides what's really there to remove.

`--locked` on the cargo path is the closest cargo analog to the sha256 pin github assets
get: it builds with upstream's published `Cargo.lock`. The npm path's bin path gets a
`.cmd` shim on Windows, since HUME's LSP transport wraps `.cmd`/`.bat` commands in
`cmd /C`.

Registering a server's languages skips any language already registered. This is what
lets a mid-session rescan leave a user's own manual `register-lsp-server!` alone instead
of last-wins-clobbering it. The registered-language check reads through the same-eval
pending op queue, so this filter is always correct in queue order regardless of load
order: the post-install rescan sees the install's own queued unregister calls the same
way, correctly re-admitting those languages instead of treating them as already taken.
The Rust side sweeps every already-open buffer for every registration this queues.

`:lsp-uninstall` takes a user-typed server name straight into a path join, so it
validates the name via `core:stdlib`'s `stdlib/safe-path-segment?` before touching disk.
`lsp-install` never needs this validation since its name always comes from the seeded
language-to-server index, never a raw argument. An orphan directory (on disk, no seeded
catalog entry) skips the unregister step and only removes the directory. Uninstall's
delete is deferred via `after! 0` so the unregister above has already shut down any
running client before the cross-process lock is acquired. The rejection of an
invalid name logs `'warn`, not `'info`: it also catches a path-traversal name (e.g.
`"../plugins"`), a security-relevant refusal worth a persistent `:messages` record, not
an ordinary usage typo (same reasoning as `core:plum`'s grammar-name rejection).

### Source rules

- **Asset format.** An asset name ending in a tar suffix (`.tar.gz`, `.tgz`, `.tar.xz`,
  `.txz`, `.tar.bz2`) is a tar archive, `.zip` a zip, `.gz` a single gzip file. Any other
  asset is a bare executable; the sync guarantees its bin path equals the asset name.
- **Toolchain kinds.** `npm`, `cargo`, `golang`, `pypi`, `gem` and `nuget` need `npm`,
  `cargo`, `go`, `python3` (`python` on Windows), `gem` and `dotnet` on `$PATH`.
- **Platforms.** A source with no `platforms` field installs everywhere.
- **Downloads.** `github` and `generic` sources both download one file per platform
  target. The resolved download is `(url asset sha bin)`, or `#f` when no row matches. A
  github row is `(target asset sha bin)` with the url derived from repo and version; a
  generic row is `(target asset url sha bin)`.
- **Compressor tools.** On Linux, `tar` shells out to `xz` or `bzip2` for those suffixes;
  macOS and Windows `tar` decompress both themselves.
- **Receipt environment.** A toolchain-installed server's run-time environment is a list
  of `("KEY" . "subpath of the server dir")` pairs; only gem installs have any.
- **Managed binary.** After a toolchain install, the binary's path under the kind's bin
  directory (relative to the server dir) must exist, or the install fails.

### System tools

Hashing, unpacking and chmod run the platform's own tools through `run-capture!` and
`run-inline-output!`, in `sha256.scm` and `unpack.scm`, rather than through hashing or
archive crates:

| Operation | macOS | Linux | Windows |
|---|---|---|---|
| sha256 | `shasum -a 256` | `sha256sum` | `certutil -hashfile … SHA256` |
| `.gz` decode | `gzip -d -f` | `gzip -d -f` | `gzip -d -f` (Git for Windows) |
| `.zip` extract | `unzip -o` | `unzip -o` | `tar -xf` |
| `.tar.*` extract | `tar -xf` | `tar -xf` (plus `xz`/`bzip2` for those suffixes) | `tar -xf` |
| exec bit | `chmod 755` | `chmod 755` | none |

An archive's regular files are found with `read-dir-iter`, which reports a symlink as a
symlink without following it, so only regular files are chmod'd and a symlinked `bin`
fails the post-unpack check instead of being made executable. A gz asset is decoded in
place next to the archive, then renamed to its `bin` path.

Zip-slip protection is the system tool's job (modern Info-ZIP strips `../` entries, bsdtar
refuses them). The sha256 pin recorded in `sources.scm` bounds the residual risk: unpacking
only runs after `lsp-install/verify-sha256!` has matched the archive against that pin.

## Server config delivery

`servers.scm`'s `config` field is delivered as **both**
`#:init-options` and `#:settings` by `register-lsp-server!`, matching Helix's own
delivery of the same blob. A catalog entry's config tail decodes to the JSON string; an
empty tail (no config) decodes to `#f`, so no config is sent.

## Install lock

`lsp-install/with-lock!` (`lock.scm`) runs a thunk under a cross-process lock
(`<data>/servers/.install-lock`), releasing it once regardless of outcome. The lock
file is created with `open-output-file`, which fails on an existing file, so creation is
atomic. A lock older than an hour is replaced with a warning; one whose mtime is in the
future (clock skew) counts as live. It is used
by both install and uninstall, so two HUME processes (or two `:lsp-install` calls) never
race the same server directory. It never re-raises the thunk's error through an outer
handler: re-raising a native-builtin error through a nested handler corrupts the Steel
VM's continuation stack (see the
[core plugins index](../../README.md#steel-pitfalls-worth-knowing-before-you-hit-them)),
so every failure path here terminates in a plain log line instead. It answers `#t` on
success, `#f` on any failure: a lock the caller couldn't acquire and a thunk that raised
both collapse to the same `#f`, indistinguishable to the caller.

The post-lock half of `:lsp-install` runs the registration rescan *outside* the lock,
after the install lock has already released it, so a failure there surfaces as a
distinct, uncaught error instead of being mislabeled "install failed" or double-releasing
the lock.

## Catalog and sources

Two separate hashes, kept intentionally apart:

| Hash | Source | Answers |
|---|---|---|
| Servers catalog | `servers.scm` | What a server actually *does* once registered: languages, command, args, config |
| Sources catalog | `sources.scm` | How to *get* it: kind, version, download targets |

Both files sit in this plugin's own directory and are read through `(plugin-dir)`, so a
fork carries its catalogs with it. A third hash, the language-to-server index, is derived from the servers catalog at load
time for O(1) language lookup. Languages are disjoint across servers by a sync-time
guarantee (`scripts/sync-grammars.py` takes only each language's primary language
server), so building that index never silently last-wins two servers against each other.

The receipt-writer's string-escaping mirrors `scripts/sync_common.py`'s own. Both must
escape the same way since receipts are read back by both Scheme and that script. One
function is the single source for the installability check, the tool preflight, and the
install-path dispatch, all keyed on the asset's archive format; another is the single
source for `:lsp-install`'s error, `:lsp-servers`'s annotation, and the discovery hint's
gate.

## Discovery hint

`on-language-set` nudges once per language per session: if a buffer's language has a
seeded server that's installable but not yet installed, it suggests `:lsp-install`. The
dedup marker (a session-scoped set) is set regardless of outcome, so a disqualified
language (no seeded server, or blocked on this platform) is never re-evaluated either.
Logged `'warn`, not `'info`: `Severity::Info` is display-only and never reaches
`:messages`, so a nudge missed at the moment it fires must stay reviewable afterward.
