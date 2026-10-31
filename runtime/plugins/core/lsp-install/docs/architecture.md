# core:lsp-install — Architecture

Two entries share one set of libraries. `plugin.scm` registers what is installed and gives
the discovery hint; `commands.scm` adds the install pipeline and the commands. They meet at
the install folders under `<data>/servers/`, which an install writes under the install lock
and the registration scan reads.

## File layout

| File | Owns |
|---|---|
| `lib/catalog.scm` | Reads `language-servers.scm` at load, and a server's `registrations/` or `requirements/` file when asked, from this plugin's `data/` directory with `(plugin-dir)`; field lookup; each language's ordered server list, its first (primary) server, and a server's languages |
| `lib/source-catalog.scm` | Reads a server's `sources/` file, which only the install pipeline needs |
| `lib/receipts.scm` | The install folders in `<data>/servers/<name>/`: which one is current, building and committing a new one, and reading a receipt |
| `lib/register.scm` | The scan that turns installed servers into registrations, and the default server list of every catalog language |
| `lib/blocker.scm` | This platform's requirements row for a server, and the check for what blocks installing it |
| `lib/install.scm` | The installers, and the per-kind choice between them |
| `lib/discovery.scm` | The discovery hint |
| `commands.scm` | `:lsp-install`, `:lsp-uninstall`, `:lsp-catalog`, the `lsp-install/install-server` command a reinstall defers to, and the completion sources `lsp:install-targets` and `lsp:servers` |
| `lib/lock.scm` | Cross-process install lock |
| `lib/unpack.scm` | Gzip decoding and chmod; the digest check and archive extraction are `core:stdlib`'s `stdlib/verify-sha256!` and `stdlib/extract-archive!` |
| `lib/platform.scm` | This platform's Mason target name, and whether it is Windows |
| `data/` | The generated catalogs and the Mason pin ([`catalogs.md`](catalogs.md)) |

## Contract with core:lsp

The two plugins share only the editor's server registry. Each installed server is
registered under its catalog name, with the command, arguments, environment and
configuration to launch it and no languages. Which servers serve a language is a separate
list, the default list the scan writes for every catalog language.

A manual `register-lsp-server!` call under the same name as a seeded server replaces what
the scan registered for it, and a rescan leaves it alone. Installing that server registers
the installed copy again, since an install unregisters the name first.

## Install folders and receipts

Each install of a server gets a new folder, `<data>/servers/<name>/<N>/`, numbered after
every other install of that server. The receipt inside it, `receipt.scm`, is the commit
point. A server's current install is its newest folder with a readable receipt, so an
install that failed or was stopped before its receipt is never current, whatever its
number, and the installed server stays current until a new install commits. The new install
is built at its final path rather than moved in from elsewhere, because a `pip` venv
records absolute paths. An entry of `servers/<name>/` whose name is not a number is not an
install.

The receipt is pure data:

```scheme
((name . "X") (version . "V") (bin . "relative/bin/path") (env-dirs ("KEY" . "subpath")…))
```

`bin` and the `env-dirs` values are relative to the install folder. Registration joins them
with it and passes the `env-dirs` as `register-lsp-server!`'s `#:env`. Only gem installs
write any, since their binstub resolves gems through `GEM_HOME` and `GEM_PATH`. The receipt
writer escapes `\` and `"` in its strings, the same rule `scripts/sync_common.py`'s
`scheme_str` uses for the `data/` files, because `read-data-file` reads receipts back as it
reads those.

## Registration scan

`lsp-install/register-installed-servers!` (`register.scm`) reads `<data>/servers/` and
registers every installed server from its current install. It is passive: no subprocess, no
network, no lock, nothing written to disk. It runs when `plugin.scm` loads, after every
install attempt, and when `:lsp-install` finds a server up to date, and it is the only
registrar for managed servers. It lists subdirectories only, so the lock file is never read
as a server. Two cases are logged as warnings and skipped: a directory with no current
install (an interrupted first install) and a directory whose name has no `registrations/`
file (an orphan, removed with `:lsp-uninstall`).

A name that already has a registration is skipped, which lets a rescan leave a user's own
`register-lsp-server!` under that name alone. `lsp-server-registered?` reads through the
same eval's pending queue, so the check holds in queue order whatever the load order: the
rescan after an install sees the install's own queued unregister and registers the name
again.

A `registrations/` record's `config` field is decoded from its JSON string each time the
server registers, and delivered as both `#:init-options` and `#:settings`. A record with no
`config` field gives `#f`, so nothing is sent.

After the registrations, the scan hands every language in `language-servers.scm` its whole
list, filters included, through `set-default-language-servers!`, whether or not any server
in it is installed. A name that is not registered matches nothing until it registers, and
the Rust side attaches every already-open buffer of the language when it does. A default
applies only while no `set-language-servers!` list is set for the language, even when the
scan runs from `:lsp-install` on the user's behalf, so a list the user set wins and may name
any registered server. A server the user registered by hand serves a language only when a
list names it. The last write to a language's default list wins, so only one plugin should
write defaults for a language.

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
- The thunk's error is not re-raised: every failure path ends in a plain log line. The
  function answers `#t` on success and `#f` on any failure, so a lock that could not be
  taken and a thunk that raised look the same to the caller.

## Implementation notes

- `lsp-install/install-blocker` (`blocker.scm`) returns the reason a server cannot be
  installed here, or `#f`. `lsp-install/row-blocker` is the same check for a row the caller
  already resolved.
- `lsp-install/target-row` (`blocker.scm`) returns the server's `requirements/` row for this
  platform, or `#f`.
- `lsp-install/package-installers` (`install.scm`) holds one `(kind installer env-dirs)` row
  per package manager; an installer takes `(name fields row dir)`, where `row` is the
  server's `target-row`.
- `lsp-install/download-row` (`install.scm`) returns this platform's download as
  `(asset url sha bin)`.
- `lsp-install/run-install!` (`install.scm`) installs a server into its folder and returns
  `(bin-rel . env-dirs)`: the binary's path inside it and the receipt's `env-dirs`.
