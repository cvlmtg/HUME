# core:lsp — Server install and registration

## Install and registration

`servers.scm` downloads, verifies, and unpacks a server (`:lsp-install`), writes a
receipt as the install commit point, and calls `registration.scm`'s scan
(`lsp/register-installed-servers!`) directly afterward so the server attaches
immediately — no cross-plugin notify, since install and registration are the same
plugin. That scan is passive (registers already-installed servers only, no subprocess,
no network) and independently reads the seeded `runtime/scheme/lsp-servers.scm` catalog
and `<data>/servers/` for receipts, registering every installed server it finds;
`plugin.scm` runs it once at its own top level, so it also happens at load or lazy
activation. It's the *only* registrar for managed servers. `:lsp-rescan-servers` exposes
the same scan for a server installed outside `:lsp-install`. A server directory with no
readable receipt — pure data, `((name . "X") (version . "V") (bin . "relative/bin/path"))`
— is treated as an interrupted install, logged as a warning naming it, rather than
silently skipped.

`registration.scm`'s catalog accessor is read-only — callers must not mutate the value it
returns, since Scheme itself enforces nothing here. Every catalog entry, in both the
servers catalog and the sources catalog, is a tagged alist tail (`(key . value)` or `(key
sub…)`, never a positional tuple), so the shared field lookup works uniformly across both
catalogs and every field shape in them.

### Install pipeline

Installing (or reinstalling) a single server always starts from a clean slate — this also
serves as the repair/upgrade path, and covers reinstalling over a running client the same
way:

1. Blocker check + tool preflight.
2. Unregister every seeded language, reaping any running client.
3. Purge any existing install (the receipt dies with it).
4. Download, verify, and unpack (github), or run `npm install`/`cargo install`.
5. Write the receipt — the commit point.
6. A `$PATH` notice, if the seeded command also happens to resolve there independently of
   the managed install.

The github path's own download step recreates the install directory right after step 3
purges it — `curl -o` needs the parent directory to already exist, and nothing else
recreates it between the purge and the download.

`:lsp-install`'s own completion source lists every language a server is seeded for (the
same set its own "no language server is seeded for" check reads against); `:lsp-uninstall`'s
lists every server with an install directory on disk, seeded or orphan alike, since its
own on-disk check — not the seeded catalog — decides what's really there to remove.

`--locked` on the cargo path is the closest cargo analog to the sha256 pin github assets
get — it builds with upstream's published `Cargo.lock`. The npm path's bin path gets a
`.cmd` shim on Windows, since HUME's LSP transport wraps `.cmd`/`.bat` commands in
`cmd /C`.

Registering a server's languages skips any language already registered — this is what
lets a mid-session rescan leave a user's own manual `register-lsp-server!` alone instead
of last-wins-clobbering it. The registered-language check reads through the same-eval
pending op queue, so this filter is always correct in queue order regardless of load
order: the post-install rescan sees the install's own queued unregister calls the same
way, correctly re-admitting those languages instead of treating them as already taken.
The Rust side sweeps every already-open buffer for every registration this queues.

`:lsp-uninstall` takes a user-typed server name straight into a path join, so it
validates the name via `core:stdlib`'s `stdlib/safe-path-segment?` before touching disk —
`lsp-install` never needs this validation since its name always comes from the seeded
language-to-server index, never a raw argument. An orphan directory (on disk, no seeded
catalog entry) skips the unregister step and only removes the directory. Uninstall's
delete is deferred via `after 0` so the unregister above has already shut down any
running client before the cross-process lock is acquired. The rejection of an
invalid name logs `'warn`, not `'info`: it also catches a path-traversal name (e.g.
`"../plugins"`) — a security-relevant refusal worth a persistent `:messages` record, not
an ordinary usage typo (same reasoning as `core:plum`'s grammar-name rejection).

## Server config delivery

`runtime/scheme/lsp-servers.scm`'s `config` field is delivered as **both**
`#:init-options` and `#:settings` by `register-lsp-server!`, matching Helix's own
delivery of the same blob. A catalog entry's config tail decodes to the JSON string; an
empty tail (no config) decodes to `#f` — no config sent.

## Install lock

`lsp/with-install-lock!` runs a thunk under a cross-process lock
(`<data>/servers/.install-lock`), releasing it exactly once regardless of outcome — used
by both install and uninstall, so two HUME processes (or two `:lsp-install` calls) never
race the same server directory. It never re-raises the thunk's error through an outer
handler: re-raising a native-builtin error through a nested handler corrupts the Steel
VM's continuation stack (see the
[core plugins index](../../README.md#steel-pitfalls-worth-knowing-before-you-hit-them)),
so every failure path here terminates in a plain log line instead. It answers `#t` on
success, `#f` on any failure — a lock the caller couldn't acquire and a thunk that raised
both collapse to the same `#f`, indistinguishable to the caller.

The post-lock half of `:lsp-install` runs the registration rescan *outside* the lock,
after the install lock has already released it, so a failure there surfaces as a
distinct, uncaught error instead of being mislabeled "install failed" or double-releasing
the lock.

## Catalog and sources

Two separate hashes, kept intentionally apart:

| Hash | Source | Answers |
|---|---|---|
| Servers catalog | `runtime/scheme/lsp-servers.scm` | What a server actually *does* once registered — languages, command, args, config |
| Sources catalog | `runtime/scheme/lsp-sources.scm` | How to *get* it — kind, version, download targets |

A third hash, the language-to-server index, is derived from the servers catalog at load
time for O(1) language lookup. Languages are disjoint across servers by a sync-time
guarantee (`scripts/sync-grammars.py` takes only each language's primary language
server), so building that index never silently last-wins two servers against each other.

The receipt-writer's string-escaping mirrors `scripts/sync_common.py`'s own — both must
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
Logged `'warn`, not `'info` — `Severity::Info` is display-only and never reaches
`:messages`, so a nudge missed at the moment it fires must stay reviewable afterward.

## Runtime management

`:lsp-status` shows every running server and its state, plus attached buffers'
diagnostic counts. `:lsp-stop [lang]`/`:lsp-restart [lang]` stop, or stop and respawn, a
running server — default the focused buffer's. All three are thin wrappers around Rust
builtins; no Scheme-side state to describe beyond the argument default.
