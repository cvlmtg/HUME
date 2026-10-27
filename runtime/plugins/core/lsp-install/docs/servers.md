# core:lsp-install — How it works

`core:lsp-install` turns a seeded catalog into language servers installed under
`<data>/servers/` and registered with the editor. This page follows that path in order:
the catalogs it starts from, how a server is installed, how an installed server becomes a
registration, the install lock, and the commands that drive it. The README covers usage,
the contract with `core:lsp`, and the catalog record formats.

## Catalogs

Files in the plugin's `data/` directory, read through `(plugin-dir)` with `read-data-file`.
Each file's shape follows how it is read. Registration (`plugin.scm`) reads all of
`language-servers.scm` at load, and one `registrations/` file per installed server. A
server's `requirements/` file is read by a requirements lookup (the discovery hint, a
blocker check), and its `sources/` file only by an install (`commands.scm`).

| Catalog | Source | Answers |
|---|---|---|
| Registrations | `registrations/<name>.scm` | What a server is registered with: command, args and config, each omitted when it is the default |
| Language lists | `language-servers.scm` | Each language's ordered servers; a server's languages are derived from it |
| Requirements | `requirements/<name>.scm` | What installing a server needs on each platform |
| Sources | `sources/<name>.scm` | How to get it: kind, version, download targets |

The first two come from the registration pin, the last two from the Mason pin, so they stay
apart. Registration, requirements and sources records are tagged alists (`(key . value)` or
`(key sub…)`, never a positional tuple), so one field lookup serves them all. A name is
looked up only when it is a file in its directory (`data-file`), so a seeded server is one
with a `registrations/` file. The language table `catalog.scm` builds is read-only: callers
must not mutate what it returns.

The language catalog is also the language-to-server index. A language lists its servers in
priority order, so `:lsp-install <lang>` installs the first and the discovery hint
looks that one up.

## Installing a server

Installing or reinstalling a server always starts from a clean slate. That makes it the
repair and upgrade path too, and it covers reinstalling over a running client:

1. Run the blocker check, which includes the required tools.
2. Unregister the server, which shuts down any running client. When an install
   already exists, the command unregisters first and runs steps 3 to 6 on the next tick, so
   the client has stopped before its files are touched. That tick runs them through
   `call!` to `lsp-install/install-server`, an `#:inline-output` command: a timer
   runs outside `:lsp-install`'s own output screen, where running an installer
   would raise.
3. Purge any existing install; the receipt goes with it.
4. Download, verify and unpack (`github`, `generic`), or run the kind's package manager
   (`npm`, `cargo`, `go`, `pip` in a venv, `gem`, `dotnet tool`).
5. Write the receipt, which is the commit point.
6. Log a `$PATH` notice if the seeded command also resolves there independently of the
   managed install.

Downloads recreate the install directory right after step 3 purges it, because `curl -o`
needs the parent to exist and nothing else recreates it in between.

A failure in step 4 or 5 deletes the install directory again before the error is raised,
so a failed install leaves nothing behind for the scan to find or for a later install to
build on.

### Choosing the source

What an install needs is decided when the catalogs are generated, not at runtime.
A server's `requirements/` file records, per platform, the download's format and the tools
the install needs on `$PATH`, so the blocker check and the discovery hint read a small file
and never load the server's `sources/` file or the installers. The installer then reads the
same row: it takes the unpack tool and the format from it instead of working them out again.
A download row in the server's `sources/` file is read once, for this platform.
A github row is `(target asset sha bin)` with the url derived from the repo and version.
A generic row is `(target asset url sha bin)`.

- **Asset format.** The generator classifies each download: a name ending in a tar suffix
  (`.tar.gz`, `.tgz`, `.tar.xz`, `.txz`, `.tar.bz2`) is a tar archive, `.zip` a zip, `.gz` a
  single gzip file. Any other asset is a bare executable; its bin path equals the asset name.
  A download in none of these shapes is dropped when `sources/` is generated.
- **Platforms.** A package-manager source with no `platforms` field installs everywhere.
- **Blockers.** One function is the single source for `:lsp-install`'s error,
  `:lsp-catalog`'s annotation and the discovery hint's gate. In the order it checks:
  unsupported platform, no install source, a stub kind, no row for this platform (not
  supported on this platform, or no prebuilt asset for it), and a missing required tool.
- **Required tools.** `npm`, `cargo`, `golang`, `pypi`, `gem` and `nuget` kinds need
  `npm`, `cargo`, `go`, `python3` (`python` on Windows), `gem` and `dotnet` on `$PATH`.
  A download needs `curl` plus the unpack tools for its format. Each is checked with
  `find-program` before anything is downloaded, so a missing tool fails naming it.

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

The archive is checked against the sha256 recorded in its `sources/` file before anything is
unpacked, and a mismatch fails the install. A gz asset is decoded in place next to the
archive, then renamed to its bin path. After extraction the bin path must exist, as a
regular file on Unix, or the install fails.

Every regular file in the extracted tree is chmod'd, not just the bin path, because a
server's layout may ship a wrapper script or sibling helpers. The tree is walked with
`read-dir-iter`, which reports a symlink as a symlink without following it, so a symlink
is neither recursed into nor chmod'd, and a symlinked bin path fails the check above.

Before anything is chmod'd, every symlink in the extracted tree must resolve inside the
server dir (`first-path-outside`). A dangling one is judged by the path it names, so it
counts too. One that points outside fails the install, since a link left behind could
later be written through. The check runs once `unzip` or `tar` has finished, so it judges
the links extraction left in the tree. A write through a link during extraction is
outside its reach, and the tool's own handling of link members is what governs it.

### Toolchain installs

After the package manager runs, the binary must exist under the kind's bin directory
(relative to the server dir), or the install fails. `cargo`, `go`, `pip`, `gem` and
`dotnet` share one check; `npm` has its own at `node_modules/.bin`. On Windows the
expected file gets a suffix: `.cmd` for npm (invoked as `npm.cmd`; a `.cmd` or `.bat`
program runs through `cmd.exe`), `.bat` for gem (invoked as
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
It is passive (no subprocess, no network). It runs at load or lazy activation and after every
install. It is the only registrar for managed servers. It lists
subdirectories only, so the lock file is never read as a server. Two cases are logged as
warnings and skipped: a directory with no readable receipt (an interrupted install) and
a directory whose name has no `registrations/` file (an orphan, to be removed with
`:lsp-uninstall`).

Each installed server registers once, under its catalog name, with the command, arguments,
environment and configuration to launch it and no languages. A name that already has a
registration is skipped, which lets a mid-session rescan leave a user's own
`register-lsp-server!` under that name alone. The check reads through the same-eval pending
queue, so it is correct in queue order regardless of load order: the rescan after an install
sees the install's own queued unregister and re-admits the name.

After the registrations, the scan sets every language's default server list. For every
language in `language-servers.scm` it calls `set-default-language-servers!` with its
whole list, filters included, whether or not any server in it is installed: a name that is
not registered matches nothing until it registers, and the Rust side attaches every
already-open buffer of the language when it does. A default applies only while no
`set-language-servers!` list is set, even when the scan runs from `:lsp-install` on the
user's behalf, so a list the user set beats it. A server the user registered by hand serves
a language only when a list names it. The last write to a language's default list wins, so
only one plugin should write defaults for a language.

A server's `config` field is delivered as both `#:init-options` and `#:settings`. The loader decodes the JSON string each time it registers a server. A record with no
`config` field decodes to `#f`, so nothing is sent.

## Install lock

`lsp-install/with-lock!` (`lock.scm`) runs a thunk under a cross-process lock at
`<data>/servers/.install-lock` and releases it once whatever the outcome. Install and
uninstall both use it, so two HUME processes, or two `:lsp-install` calls, never work on
the same server directory at once.

- The lock is the OS lock `try-lock-file!` takes on that file, not the file itself, which
  stays on disk. Taking it is atomic: while one HUME holds it, another's attempt fails.
- Nothing has to recover a lock left behind: the OS releases it when the HUME holding it
  exits, and the eval that took it releases it when it ends. An install the script budget
  stopped runs no cleanup, so the second rule is what frees its lock.
- The thunk's error is not re-raised: every failure path ends in a plain log line. The function answers `#t` on success and
  `#f` on any failure, so a lock that could not be taken and a thunk that raised look the
  same to the caller.

## Commands

**`:lsp-install`** takes a seeded server name or a language. A name that is a seeded server
installs that server; anything else is a language, installing its first server, and no argument
means the buffer's language. A name that is both a server and a language must be its own
language's first server, so the two readings agree. Completion offers every server with an
install source, including one that cannot be installed here (`:lsp-install` then reports why),
and every seeded language. If the receipt's version already matches the
seeded one it logs "up to date" and only runs the scan. Otherwise it installs under the
lock and then runs the scan outside the lock, so a failure in the scan surfaces as its own
error instead of being reported as a failed install.

**`:lsp-uninstall`** takes a user-typed server name straight into a path join, so it first
validates it with `safe-path-segment?`. `:lsp-install` needs no such
check, since the catalog lookup (`data-file`) rejects a name that is not a safe segment. An invalid name logs
`'error`, as any typed argument the command can't act on does, so a path-traversal attempt
such as `"../plugins"` stays in `:messages` (see the index's
[Log severity](../../README.md#log-severity)). The command unregisters the server, then
removes its directory. An orphan (on disk, with no `registrations/` file) skips
the unregister step. The removal is deferred with `after! 0` so the unregister has shut
down any running client before the lock is taken. Its completion lists every server with a
directory on disk, seeded or orphan, because the on-disk check decides what there is to
remove.

**`:lsp-catalog`** shows one `stdlib/report-section` in `[lsp-catalog]`, with one line per
seeded server sorted by name: its name, its languages, and either `installed vX`,
`installed vX — update available (vY)`, the install blocker, or `not installed`.

### Discovery hint

`on-language-set` nudges once per language per session. It fires when the language has a
seeded server with no blocker, no receipt, and no registration already in place, so a
user's own registration suppresses it. The suggestion is to run `:lsp-install`. The dedup
marker is set whatever the outcome, so a language that fails any of those tests is not
re-evaluated. The nudge logs `'warning` because `'info` never reaches `:messages` (see the
[Log severity](../../README.md#log-severity) section), and a missed nudge should stay
reviewable.
