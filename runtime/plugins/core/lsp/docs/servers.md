# core:lsp — Server install and registration

## Install and registration

`servers.scm` downloads, verifies, and unpacks a server (`:lsp-install`), writes a
receipt as the install commit point, and calls `registration.scm`'s scan
(`lsp/register-installed-servers!`) directly afterward so the server attaches
immediately — no cross-plugin notify, since install and registration are the same
plugin. That scan independently reads the seeded `runtime/scheme/lsp-servers.scm`
catalog and `<data>/servers/` for receipts, registering (`register-lsp-server!`) every
installed server it finds; `plugin.scm` runs it once at its own top level, so it also
happens at load or lazy activation. It's the *only* registrar for managed servers.
`:lsp-rescan-servers` exposes the same scan for a server installed outside
`:lsp-install`.

Installing (or reinstalling) a single server always starts from a clean slate — this
also serves as the repair/upgrade path, and covers reinstalling over a running client
the same way: (1) blocker check + tool preflight; (2) unregister every seeded
language, reaping any running client; (3) purge any existing install (the receipt dies
with it); (4) download, verify, and unpack (github), or run `npm install`/`cargo
install`; (5) write the receipt — the commit point; (6) a `$PATH` notice, if the
seeded command also happens to resolve there independently of the managed install.

`--locked` on the cargo path is the closest cargo analog to the sha256 pin github
assets get — it builds with upstream's published `Cargo.lock`. The npm path's bin
path gets a `.cmd` shim on Windows, since HUME's LSP transport wraps `.cmd`/`.bat`
commands in `cmd /C` (cfg-gated).

Registering a server's languages skips any language already registered — this is what
lets a mid-session rescan leave a user's own manual `register-lsp-server!` alone
instead of last-wins-clobbering it. `lsp-registered-for-language?` reads through the
same-eval pending op queue, so this filter is always correct in queue order regardless
of load order: the post-install rescan sees `lsp/install-server!`'s own queued
`unregister-lsp-server!` calls the same way, correctly re-admitting those languages
instead of treating them as already taken. `apply_pending_lsp_server_reg`
(`hume-editor/src/editor/lsp/registry.rs`) sweeps every already-open buffer on the
Rust side for every registration this queues.

`:lsp-uninstall` takes a user-typed server name straight into a path join, so it
validates the name (non-empty, not `.`/`..`, no path separators) before touching disk;
`lsp-install` never needs this validation since its name always comes from the seeded
language-to-server index, never a raw argument. An orphan directory (on disk, no
seeded catalog entry) skips the unregister step and only removes the directory.
Uninstall's delete is deferred to `after 0` so the unregister above has already shut
down any running client before the cross-process lock is acquired. The rejection of an
invalid name logs `'warn`, not `'info`: it also catches a path-traversal name (e.g.
`"../plugins"`) — a security-relevant refusal worth a persistent `:messages` record,
not an ordinary usage typo (same reasoning as `core:plum`'s `fetch-raw-query` grammar-
name rejection).

## Server config delivery

`runtime/scheme/lsp-servers.scm`'s `config` field is delivered as **both**
`#:init-options` and `#:settings` by `register-lsp-server!`, matching Helix's own
delivery of the same blob. A catalog entry's `(config . "json")` tail decodes to the
JSON string; the empty tail `(config)` (no config) decodes to `'()`, read here as `#f`
— no config sent.

## Install lock

`lsp/with-install-lock!` (`servers.scm`) runs a thunk under a cross-process lock
(`<data>/servers/.install-lock`), releasing it exactly once regardless of outcome —
used by both install and uninstall, so two HUME processes (or two `:lsp-install`
calls) never race the same server directory. It never re-raises the thunk's error
through an outer `with-handler`: re-raising a native-builtin error through a nested
handler corrupts the Steel VM's continuation stack, so every failure path here
terminates in a plain `log!` instead. Returns `#t` on success, `#f` on any failure — a
lock the caller couldn't acquire and a thunk that raised both collapse to the same
`#f`, indistinguishable to the caller.

`lsp/lsp-install-or-report!` — the post-lock half of `:lsp-install` — runs the
registration rescan *outside* the lock, after `with-install-lock!` has already
released it, so a failure there surfaces as a distinct, uncaught error instead of
being mislabeled "install failed" or double-releasing the lock.

## Catalog and sources

Two separate hashes, kept intentionally apart: `registration.scm`'s `*lsp-servers*`
(from `runtime/scheme/lsp-servers.scm` — languages, command, args, config per server)
is what a server actually *does* once registered; `servers.scm`'s `*lsp-sources*`
(from `runtime/scheme/lsp-sources.scm` — kind, version, download targets) is how to
*get* it. A third hash, `*lsp-lang->server*`, is derived from the servers catalog at
load time for O(1) language lookup. Languages are disjoint across servers by a
sync-time guarantee — `scripts/sync-grammars.py` takes only each language's primary
language server — so building that index never silently last-wins two servers against
each other.

`lsp/scheme-quote` (receipt writing) mirrors `scripts/sync_common.py`'s `scheme_str` —
both must escape the same way since receipts are read back by both Scheme and that
script. `lsp/asset-format` is the single source for the installability check, the
tool preflight, and the install-path dispatch; `lsp/install-blocker` is the single
source for `:lsp-install`'s error, `:lsp-servers`'s annotation, and the discovery
hint's gate.

## Discovery hint

`on-language-set` nudges once per language per session: if a buffer's language has a
seeded server that's installable but not yet installed, it suggests `:lsp-install`.
The dedup marker is set regardless of outcome, so a disqualified language (no seeded
server, or blocked on this platform) is never re-evaluated either. Logged `'warn`, not
`'info` — `Severity::Info` is display-only and never reaches `:messages`, so a nudge
missed at the moment it fires must stay reviewable afterward.

## Runtime management

`:lsp-status` shows every running server and its state, plus attached buffers'
diagnostic counts (`lsp-show-status!`). `:lsp-stop [lang]`/`:lsp-restart [lang]` stop,
or stop and respawn, a running server — default the focused buffer's — via
`lsp-stop!`/`lsp-restart!`. All three are thin wrappers around Rust builtins; no
Scheme-side state to describe beyond the argument default.
