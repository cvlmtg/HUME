# core:lsp-install — How it works

`core:lsp-install` turns a seeded catalog into language servers that are installed under
`<data>/servers/` and registered with the editor. This page follows that path in order:
the catalogs it starts from, how a server is installed, how an installed server becomes a
registration, then the commands, the lock and the config handling around it. The README
covers usage and the record formats; the wider rationale (why Helix and Mason, why
receipts) is in `docs/LSP-INSTALL.md`.

## The contract with `core:lsp`

There is none. The plugin registers servers through `register-lsp-server!` and removes
them through `unregister-lsp-server!`, the editor-level registry that `core:lsp` and every
other plugin share. `core:lsp` never asks this plugin anything, so a replacement installer
needs nothing beyond those two builtins, and a manual `register-lsp-server!` call always
wins over what this plugin registers.

## Catalogs

Two files in the plugin's own directory, read once at load through `(plugin-dir)` so a
fork carries its catalogs with it:

| Hash | Source | Answers |
|---|---|---|
| Servers catalog | `servers.scm` | What a server *does* once registered: languages, command, args, config |
| Sources catalog | `sources.scm` | How to *get* it: kind, version, download targets |

They are kept apart because they come from different upstreams and different pins; see the
README for the record shapes. Every entry in both is a tagged alist tail (`(key . value)`
or `(key sub…)`, never a positional tuple), so one field lookup serves both catalogs and
every field shape in them. `catalog.scm`'s accessors are read-only: callers must not mutate
what they return, since Scheme enforces nothing here.

A third hash, the language-to-server index, is derived from the servers catalog at load
time. Languages are disjoint across servers by a sync-time guarantee
(`scripts/sync-grammars.py` takes only each language's primary language server), so
building it never silently last-wins two servers against each other. `:lsp-install` and the
discovery hint both look a language up there.

## Installing a server

Installing, or reinstalling, a single server always starts from a clean slate. That also
makes it the repair and upgrade path, and it covers reinstalling over a running client:

1. Blocker check and tool preflight.
2. Unregister every seeded language, reaping any running client.
3. Purge any existing install; the receipt dies with it.
4. Download, verify and unpack (`github`, `generic`), or run the kind's package manager
   (`npm`, `cargo`, `go`, `pip` in a venv, `gem`, `dotnet tool`).
5. Write the receipt, which is the commit point.
6. Log a `$PATH` notice if the seeded command also resolves there independently of the
   managed install.

The github download step recreates the install directory right after step 3 purges it:
`curl -o` needs the parent to exist and nothing else recreates it in between.

### Choosing the source

One function resolves a download for this platform as `(url asset sha bin)`, or `#f` when
no row matches. A github row is `(target asset sha bin)` with the url derived from repo
and version; a generic row is `(target asset url sha bin)`. That one result serves the
installability check, the tool preflight and the install itself.

- **Asset format.** A name ending in a tar suffix (`.tar.gz`, `.tgz`, `.tar.xz`, `.txz`,
  `.tar.bz2`) is a tar archive, `.zip` a zip, `.gz` a single gzip file. Any other asset is
  a bare executable; the sync guarantees its bin path equals the asset name.
- **Platforms.** A source with no `platforms` field installs everywhere.
- **Toolchain kinds.** `npm`, `cargo`, `golang`, `pypi`, `gem` and `nuget` need `npm`,
  `cargo`, `go`, `python3` (`python` on Windows), `gem` and `dotnet` on `$PATH`. One table
  maps each kind to its tool, shared by the blocker and the preflight.
- **Blockers.** One function is the single source for `:lsp-install`'s error,
  `:lsp-servers`'s annotation and the discovery hint's gate: unsupported platform, no
  install source, platform-restricted, missing toolchain, stub kind, or no prebuilt asset.

### Verifying and unpacking

Hashing, unpacking and chmod run the platform's own tools through `run-capture!` and
`run-inline-output!` (`sha256.scm`, `unpack.scm`), not through hashing or archive crates:

| Operation | macOS | Linux | Windows |
|---|---|---|---|
| sha256 | `shasum -a 256` | `sha256sum` | `certutil -hashfile … SHA256` |
| `.gz` decode | `gzip -d -f` | `gzip -d -f` | `gzip -d -f` (Git for Windows) |
| `.zip` extract | `unzip -o` | `unzip -o` | `tar -xf` |
| `.tar.*` extract | `tar -xf` | `tar -xf`, plus `xz`/`bzip2` for those suffixes | `tar -xf` |
| exec bit | `chmod 755` | `chmod 755` | none |

macOS and Windows `tar` decompress xz and bzip2 themselves; Linux `tar` shells out to the
compressor, so the preflight also asks for it.

The archive is checked against the sha256 recorded in `sources.scm` before anything is
unpacked, and deleted on a mismatch. Zip-slip protection is the system tool's job (modern
Info-ZIP strips `../` entries, bsdtar refuses them); the pin bounds the residual risk,
since only a hash-locked asset ever reaches the unpacker.

Every regular file in the extracted tree is chmod'd, not just the seeded bin path, because
a server's layout may ship a wrapper script or sibling helpers. The tree is walked with
`read-dir-iter`, which reports a symlink as a symlink without following it, so a symlink
is neither recursed into nor chmod'd and one that points outside the server dir cannot have
its target's permissions changed. A symlinked bin path fails the post-unpack check. A gz
asset is decoded in place next to the archive, then renamed to its bin path.

### Toolchain installs

After a package manager runs, one shared check confirms the expected binary exists under
the kind's bin directory (relative to the server dir), or the install fails. `--locked` on
the cargo path is the closest analog to the sha256 pin github assets get: it builds with
upstream's published `Cargo.lock`. The npm bin path gets a `.cmd` shim on Windows, since
HUME's LSP transport wraps `.cmd`/`.bat` commands in `cmd /C`.

## Receipts and registration

The receipt, `<data>/servers/<name>/receipt.scm`, is pure data:

```scheme
((name . "X") (version . "V") (bin . "relative/bin/path") (env-dirs ("KEY" . "subpath")…))
```

`env-dirs` values are relative to the server dir; registration joins them with it and
passes the result as `register-lsp-server!`'s `#:env`. Only gem installs write any, since
their binstub resolves gems through `GEM_HOME` and `GEM_PATH`. The receipt writer's string
escaping mirrors `scripts/sync_common.py`'s, because receipts are read back by both Scheme
and that script.

The scan in `register.scm` reads `<data>/servers/` and registers every installed server.
It is passive (no subprocess, no network) and runs once at load or lazy activation, after
every install, and on demand as `:lsp-rescan-servers`, which also picks up a server
installed outside `:lsp-install`. It is the only registrar for managed servers. A server
directory with no readable receipt is an interrupted install and is logged as a warning
naming it rather than skipped silently.

Registering a server's languages skips any language already registered. That is what lets
a mid-session rescan leave a user's own `register-lsp-server!` alone instead of
last-wins-clobbering it. The check reads through the same-eval pending queue, so it is
correct in queue order regardless of load order: the post-install rescan sees the install's
own queued unregister calls and correctly re-admits those languages. The Rust side sweeps
every already-open buffer for each registration this queues.

## Commands

`:lsp-install` completes from every language a server is seeded for, the same set its "no
language server is seeded for" check reads. If the receipt's version already matches the
seeded one it logs "up to date" and only re-runs the scan. Otherwise it installs under the
lock, then runs the scan *outside* the lock, so a failure in the scan surfaces as its own
uncaught error instead of being mislabeled "install failed" or releasing the lock twice.

`:lsp-uninstall` takes a user-typed server name straight into a path join, so it first
validates it with `core:stdlib`'s `stdlib/safe-path-segment?`. `:lsp-install` never needs
that, since its name always comes from the language-to-server index. The rejection logs
`'warn`, not `'info`: it also catches a path-traversal name such as `"../plugins"`, a
security-relevant refusal that deserves a persistent `:messages` record rather than being
treated as a usage typo. Its completion lists every server with a directory on disk,
seeded or orphan, because the on-disk check decides what there is to remove. An orphan
(on disk, no seeded entry) skips the unregister step and only has its directory removed.
The delete is deferred with `after! 0`, so the unregister has already shut down any running
client before the lock is taken.

`:lsp-servers` lists every seeded server with its languages, seeded and installed version,
and either its install status or its blocker.

### Discovery hint

`on-language-set` nudges once per language per session: if the language has a seeded
server that is installable but not installed, it suggests `:lsp-install`. The dedup marker
is set whatever the outcome, so a language with no seeded server, or one blocked on this
platform, is never re-evaluated. It logs `'warn`, not `'info`: `Severity::Info` is
display-only and never reaches `:messages`, so a nudge missed when it fires must stay
reviewable afterwards.

## Install lock

`lsp-install/with-lock!` (`lock.scm`) runs a thunk under a cross-process lock at
`<data>/servers/.install-lock`, releasing it once whatever the outcome. Install and
uninstall both use it, so two HUME processes, or two `:lsp-install` calls, never race the
same server directory. The scan skips the lock file, so it is never read as a server.

- The file is created with `open-output-file`, which fails on an existing file, so creation
  is atomic.
- A lock older than an hour is replaced with a warning. One whose mtime is in the future
  (clock skew) counts as live.
- The thunk's error is never re-raised through an outer handler: re-raising a native
  builtin error through a nested handler corrupts the Steel VM's continuation stack (see
  the [core plugins index](../../README.md#steel-pitfalls-worth-knowing-before-you-hit-them)).
  Every failure path ends in a plain log line, and the function answers `#t` on success
  and `#f` on any failure, so a lock that could not be taken and a thunk that raised are
  indistinguishable to the caller.

## Config delivery

A server's `config` field in `servers.scm` is delivered as both `#:init-options` and
`#:settings` by `register-lsp-server!`, matching Helix's own delivery of the same blob. The
config tail decodes to a JSON string; an empty tail decodes to `#f`, so no config is sent.
