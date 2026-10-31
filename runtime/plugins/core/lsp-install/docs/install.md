# core:lsp-install — Installing servers

How `commands.scm` installs a server, the checks around an install, and the commands and
hint that drive it. Install folders, receipts, the registration scan and the lock are in
[`architecture.md`](architecture.md); the records an install reads are in
[`catalogs.md`](catalogs.md).

## Installing a server

`lsp-install/install-server!` (`install.scm`) runs under the install lock. Installing over
an existing install is the repair and upgrade path too.

1. Run the blocker check, which includes the required tools, and raise its reason if
   something blocks the install.
2. Unregister the server, which shuts down any running client.
3. Create the new install folder.
4. Download, verify and unpack (`github`, `generic`), or run the kind's package manager
   (`npm`, `cargo`, `go`, `pip` in a venv, `gem`, `dotnet tool`), into that folder.
5. Write the receipt, then delete every other entry of `servers/<name>/`: the older
   install, and folders that stopped installs left. Each delete that fails is a warning,
   and the next install of that server tries again.
6. Log a `$PATH` notice if the seeded command also resolves there independently of the
   managed install.

A failure in step 3 or 4, or in writing the receipt, deletes the new folder, and
`servers/<name>/` when nothing else is in it, before the error is raised. A delete that
fails is logged with the folder's path, and the original error is raised either way. The
command registers the installed servers whether or not the install succeeded, so a kept
server starts again, and when the install did not succeed and the server has a current
install it logs that the installed version was kept.

A first install runs these steps from `:lsp-install` itself. A reinstall over a running
client cannot: the client has to stop before the install touches its files. When the
server has a current install, `:lsp-install` unregisters it at once, with no blocker check,
and runs the steps on the next tick through `call!` to `lsp-install/install-server`, an
`#:inline-output` command: a timer runs outside `:lsp-install`'s own output screen, where
running an installer would raise. Step 2 then repeats the unregister. A reinstall that
step 1 blocks has therefore already unregistered the server, and the scan that follows
every attempt registers the kept install again.

### Choosing the source

What an install needs is decided when the catalogs are generated, not at runtime
([Generation](catalogs.md#generation)). A server's `requirements/` file records, per
platform, the download's format and the tools the install needs on `$PATH`, so the blocker
check and the discovery hint read a small file and never load the server's `sources/` file
or the installers. The installer reads the same row for the download's format, and a pypi
installer for its interpreter. The extractor for a tar or zip download is
`stdlib/extract-archive!`'s choice, through the same `stdlib/find-extractor` lookup the
check uses, and a gz download is always decoded with `gzip`.

A download row in the server's `sources/` file is read once, for this platform. A github
row is `(target asset sha bin)`, with the url derived from the repo and version. A generic
row is `(target asset url sha bin)`.

- **Platforms.** A package-manager source with no `platforms` field installs everywhere.
- **Blockers.** One function is the single source for `:lsp-install`'s error,
  `:lsp-catalog`'s annotation and the discovery hint's gate. In the order it checks:
  unsupported platform, no install source, a stub kind, no row for this platform (not
  supported on this platform, or no prebuilt asset for it), and a missing required tool.
- **Required tools.** `npm`, `cargo`, `golang`, `pypi`, `gem` and `nuget` kinds need
  `npm`, `cargo`, `go`, `python3` (`python` on Windows), `gem` and `dotnet` on `$PATH`. A
  download needs `curl` plus the unpack tools for its format. Each is checked before
  anything is downloaded, so a missing tool fails naming it.

### Verifying and unpacking

Hashing, unpacking and chmod run the platform's own tools through `run-capture!` and
`run-inline-output!` (`core:stdlib`'s `stdlib/verify-sha256!` and
`stdlib/extract-archive!`, then `unpack.scm`):

| Operation | macOS | Linux | Windows |
|---|---|---|---|
| sha256 | `shasum -a 256` | `sha256sum` | `certutil -hashfile … SHA256` |
| `.gz` decode | `gzip -d -f` | `gzip -d -f` | `gzip -d -f` (Git for Windows) |
| `.zip` extract | `unzip -o` | `unzip -o` | `tar -xf` |
| `.tar.*` extract | `tar -xf` | `tar -xf`, plus `xz`/`bzip2` for those suffixes | `tar -xf` |
| exec bit | `chmod 755` | `chmod 755` | none |

macOS and Windows `tar` decompress xz and bzip2 themselves. Linux `tar` shells out to the
compressor, so that tool is also required.

The archive is checked against the sha256 recorded in its `sources/` file before anything
is unpacked, and a mismatch fails the install. A gz asset is decoded in place next to the
archive, then renamed to its bin path. After extraction the bin path must exist, or the
install fails. The check is `path-exists?`, which follows a symlink and accepts a
directory.

Every regular file in the extracted tree is chmod'd, not just the bin path, because a
server's layout may ship a wrapper script or sibling helpers. The tree is walked with
`read-dir-iter`, which reports a symlink as a symlink without following it, so a symlink
is neither recursed into nor chmod'd.

Before anything is chmod'd, every symlink in the extracted tree must resolve inside the
install folder (`first-path-outside`, in `stdlib/extract-archive!`). A dangling one is
judged by the path it names, so it counts too. One that points outside fails the install,
since a link left behind could later be written through. The check runs once `unzip` or
`tar` has finished, so it judges the links extraction left in the tree. A write through a
link during extraction is outside its reach, and the tool's own handling of link members
is what governs it.

### Toolchain installs

After the package manager runs, the binary must exist under the kind's bin directory
(relative to the install folder), or the install fails. Every kind shares one check,
`lsp-install/managed-bin!`, which takes the bin directory: `node_modules/.bin` for npm,
`venv/Scripts` (Windows) or `venv/bin` for pip, and `bin` for the others. On Windows the
expected file gets a suffix: `.cmd` for npm (invoked as `npm.cmd`; a `.cmd` or `.bat`
program runs through `cmd.exe`), `.bat` for gem (invoked as `gem.cmd`), and `.exe` for the
others. The cargo path passes `--locked`, the closest analog to the sha256 pin that github
assets get: it builds with upstream's published `Cargo.lock`.

## Commands

**`:lsp-install`** takes a seeded server name or a language. A name that is a seeded server
installs that server; anything else is a language, installing its first server, and no
argument means the buffer's language. A name that is both a server and a language must be
its own language's first server, so the two readings agree. Completion offers every server
with an install source, including one that cannot be installed here (`:lsp-install` then
reports why), and every seeded language. If the current install's receipt version already
matches the seeded one it logs "up to date" and only runs the scan. Otherwise it installs
under the lock, as under [Installing a server](#installing-a-server), and then runs the
scan outside the lock, so a failure in the scan surfaces as its own error instead of being
reported as a failed install.

**`:lsp-uninstall`** takes a user-typed server name straight into a path join, so it first
validates it with `safe-path-segment?`. `:lsp-install` needs no such check, since the
catalog lookup (`data-file`) rejects a name that is not a safe segment. An invalid name
logs `'error`, as any typed argument the command can't act on does, so a path-traversal
attempt such as `"../plugins"` stays in `:messages` (see the index's
[Log severity](../../README.md#log-severity)). The command unregisters the server, then
removes its directory. An orphan (on disk, with no `registrations/` file) skips the
unregister step. The removal is deferred with `after! 0` so the unregister has shut down
any running client before the lock is taken. Its completion lists every server with a
directory on disk, seeded or orphan, because the on-disk check decides what there is to
remove.

**`:lsp-catalog`** shows one `stdlib/report-section` in `[lsp-catalog]`, with one line per
seeded server sorted by name: its name, its languages, and either `installed vX`,
`installed vX — update available (vY)`, the install blocker, or `not installed`.

### Discovery hint

`on-language-set` nudges once per language per session. It fires when the language's first
server has no blocker and no current install, and the language's server list
(`lsp-language-servers`, which keeps only registered servers) is empty. A server the user
registered by hand therefore suppresses it only when that list names the server. The
suggestion is to run `:lsp-install`. The dedup marker is set whatever the outcome, so a
language that fails any of those tests is not re-evaluated. The nudge logs `'warning`
because `'info` never reaches `:messages` (see the index's
[Log severity](../../README.md#log-severity)), and a missed nudge should stay reviewable.
