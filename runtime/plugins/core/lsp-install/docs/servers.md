# core:lsp-install — How it works

`core:lsp-install` turns a seeded catalog into language servers installed under
`<data>/servers/` and registered with the editor. This page follows that path in order:
the catalogs it starts from, how a server is installed, how an installed server becomes a
registration, the install lock, and the commands that drive it. The README covers usage,
the contract with `core:lsp`, and the catalog record formats.

## Catalogs

Two files in the plugin's directory, read once at load through `(plugin-dir)`:

| Hash | Source | Answers |
|---|---|---|
| Servers catalog | `servers.scm` | What a server does once registered: languages, command, args, config |
| Sources catalog | `sources.scm` | How to get it: kind, version, download targets |

They stay apart because they come from different upstreams and different pins. Every entry
in both is a tagged alist tail (`(key . value)` or `(key sub…)`, never a positional
tuple), so one field lookup serves both. The hashes `catalog.scm` exposes are read-only:
callers must not mutate what they return.

A third hash, the language-to-server index, is derived from the servers catalog at load.
Languages are disjoint across servers by a sync-time guarantee (`scripts/sync-grammars.py`
takes only each language's primary language server), so the index never has two servers
competing for one language. `:lsp-install` and the discovery hint both look languages up
there.

## Installing a server

Installing or reinstalling a server always starts from a clean slate. That makes it the
repair and upgrade path too, and it covers reinstalling over a running client:

1. Resolve the install plan: blocker check, which includes the required tools.
2. Unregister every seeded language, which shuts down any running client.
3. Purge any existing install; the receipt goes with it.
4. Download, verify and unpack (`github`, `generic`), or run the kind's package manager
   (`npm`, `cargo`, `go`, `pip` in a venv, `gem`, `dotnet tool`).
5. Write the receipt, which is the commit point.
6. Log a `$PATH` notice if the seeded command also resolves there independently of the
   managed install.

Downloads recreate the install directory right after step 3 purges it, because `curl -o`
needs the parent to exist and nothing else recreates it in between.

### Choosing the source

One function resolves a source into a plan: the tools the kind needs on `$PATH`, the env
dirs its receipt records, and the procedure that installs into a directory. It is the only
place that branches on the source's kind. A download plan reads this platform's row once.
A github row is `(target asset sha bin)` with the url derived from the repo and version.
A generic row is `(target asset url sha bin)`. That one plan serves the installability
check, the tool check and the install itself.

- **Asset format.** A name ending in a tar suffix (`.tar.gz`, `.tgz`, `.tar.xz`, `.txz`,
  `.tar.bz2`) is a tar archive, `.zip` a zip, `.gz` a single gzip file. Any other asset is
  a bare executable; the sync guarantees its bin path equals the asset name.
- **Platforms.** A source with no `platforms` field installs everywhere.
- **Blockers.** One function is the single source for `:lsp-install`'s error,
  `:lsp-servers`'s annotation and the discovery hint's gate. In the order it checks:
  unsupported platform, no install source, not supported on this platform, a stub kind,
  no prebuilt asset for this platform, an unrecognised asset format, and a missing
  required tool.
- **Required tools.** `npm`, `cargo`, `golang`, `pypi`, `gem` and `nuget` kinds need
  `npm`, `cargo`, `go`, `python3` (`python` on Windows), `gem` and `dotnet` on `$PATH`.
  A download needs `curl` plus the unpack tools for its format. Each is checked with
  `which` before anything is downloaded, so a missing tool fails naming it.

### Verifying and unpacking

Hashing, unpacking and chmod run the platform's own tools through `run-capture!` and
`run-inline-output!` (`sha256.scm`, `unpack.scm`):

| Operation | macOS | Linux | Windows |
|---|---|---|---|
| sha256 | `shasum -a 256` | `sha256sum` | `certutil -hashfile … SHA256` |
| `.gz` decode | `gzip -d -f` | `gzip -d -f` | `gzip -d -f` (Git for Windows) |
| `.zip` extract | `unzip -o` | `unzip -o` | `tar -xf` |
| `.tar.*` extract | `tar -xf` | `tar -xf`, plus `xz`/`bzip2` for those suffixes | `tar -xf` |
| exec bit | `chmod 755` | `chmod 755` | none |

macOS and Windows `tar` decompress xz and bzip2 themselves. Linux `tar` shells out to the
compressor, so that tool is also required.

The archive is checked against the sha256 recorded in `sources.scm` before anything is
unpacked, and is deleted on a mismatch. A gz asset is decoded in place next to the
archive, then renamed to its bin path. After extraction the bin path must exist, as a
regular file on Unix, or the install fails.

Every regular file in the extracted tree is chmod'd, not just the bin path, because a
server's layout may ship a wrapper script or sibling helpers. The tree is walked with
`read-dir-iter`, which reports a symlink as a symlink without following it. A symlink is
neither recursed into nor chmod'd, so one that points outside the server dir cannot have
its target's permissions changed, and a symlinked bin path fails the check above.

### Toolchain installs

After the package manager runs, the binary must exist under the kind's bin directory
(relative to the server dir), or the install fails. `cargo`, `go`, `pip`, `gem` and
`dotnet` share one check; `npm` has its own at `node_modules/.bin`. On Windows the
expected file gets a suffix: `.cmd` for npm (invoked as `npm.cmd`, and HUME's LSP
transport wraps `.cmd` and `.bat` commands in `cmd /C`), `.bat` for gem (invoked as
`gem.cmd`), and `.exe` for the others. The pip venv's bin directory is `venv/Scripts` on
Windows and `venv/bin` elsewhere. The cargo path passes `--locked`, the closest analog to
the sha256 pin that github assets get: it builds with upstream's published `Cargo.lock`.

## Receipts and registration

The receipt, `<data>/servers/<name>/receipt.scm`, is pure data:

```scheme
((name . "X") (version . "V") (bin . "relative/bin/path") (env-dirs ("KEY" . "subpath")…))
```

`env-dirs` values are relative to the server dir. Registration joins them with it and
passes the result as `register-lsp-server!`'s `#:env`. Only gem installs write any, since
their binstub resolves gems through `GEM_HOME` and `GEM_PATH`. The receipt writer's string
escaping mirrors `scripts/sync_common.py`'s, because receipts are read back by both Scheme
and that script.

The scan in `register.scm` reads `<data>/servers/` and registers every installed server.
It is passive (no subprocess, no network). It runs at load or lazy activation, after every
install, and on demand as `:lsp-rescan-servers`, which also picks up a server installed
outside `:lsp-install`. It is the only registrar for managed servers. It lists
subdirectories only, so the lock file is never read as a server. Two cases are logged as
warnings and skipped: a directory with no readable receipt (an interrupted install) and
a directory whose name is not in `servers.scm` (an orphan, to be removed with
`:lsp-uninstall`).

Registering a server's languages skips any language that already has a registration. That
lets a mid-session rescan leave a user's own `register-lsp-server!` alone. The check reads
through the same-eval pending queue, so it is correct in queue order regardless of load
order: the rescan after an install sees the install's own queued unregister calls and
re-admits those languages. The Rust side sweeps every already-open buffer for each
registration this queues.

A server's `config` field is delivered as both `#:init-options` and `#:settings`, matching
Helix. The loader decodes the JSON string each time it registers a server. An empty config
tail decodes to `#f`, so nothing is sent.

## Install lock

`lsp-install/with-lock!` (`lock.scm`) runs a thunk under a cross-process lock at
`<data>/servers/.install-lock` and releases it once whatever the outcome. Install and
uninstall both use it, so two HUME processes, or two `:lsp-install` calls, never work on
the same server directory at once.

- The file is created with `open-output-file`, which fails on an existing file, so creation
  is atomic.
- A lock older than an hour is replaced with a warning. One whose mtime is in the future
  (clock skew) counts as live.
- The thunk's error is never re-raised through an outer handler: re-raising a native
  builtin error through a nested handler corrupts the Steel VM's continuation stack (see
  the [core plugins index](../../README.md#steel-pitfalls-worth-knowing-before-you-hit-them)).
  Every failure path ends in a plain log line. The function answers `#t` on success and
  `#f` on any failure, so a lock that could not be taken and a thunk that raised look the
  same to the caller.

## Commands

**`:lsp-install`** completes from every language a server is seeded for, the same set its
"no language server is seeded for" check reads. If the receipt's version already matches the
seeded one it logs "up to date" and only runs the scan. Otherwise it installs under the
lock and then runs the scan outside the lock, so a failure in the scan surfaces as its own
error instead of being reported as a failed install.

**`:lsp-uninstall`** takes a user-typed server name straight into a path join, so it first
validates it with `core:stdlib`'s `stdlib/safe-path-segment?`. `:lsp-install` needs no such
check, since its name always comes from the language-to-server index. An invalid name logs
`'warn` because it can be a path-traversal attempt such as `"../plugins"`, which deserves a
persistent record (see the index's
[Log severity](../../README.md#log-severity)). The command unregisters the server's
languages, then removes its directory. An orphan (on disk, not in `servers.scm`) skips
the unregister step. The removal is deferred with `after! 0` so the unregister has shut
down any running client before the lock is taken. Its completion lists every server with a
directory on disk, seeded or orphan, because the on-disk check decides what there is to
remove.

**`:lsp-servers`** prints one line per seeded server: its name, its languages, and either
`installed vX`, `installed vX — update available (vY)`, the install blocker, or `not
installed`. A count line follows.

**`:lsp-rescan-servers`** runs the scan.

### Discovery hint

`on-language-set` nudges once per language per session. It fires when the language has a
seeded server with no blocker, no receipt, and no registration already in place, so a
user's own registration suppresses it. The suggestion is to run `:lsp-install`. The dedup
marker is set whatever the outcome, so a language that fails any of those tests is not
re-evaluated. The nudge logs `'warn` because `'info` never reaches `:messages` (see the
[Log severity](../../README.md#log-severity) section), and a missed nudge should stay
reviewable.
