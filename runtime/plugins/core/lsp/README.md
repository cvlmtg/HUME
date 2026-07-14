# core:lsp

Language server features: hover, go-to-definition (+ declaration / type-definition /
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints. Also owns the LSP server lifecycle end to
end — install, uninstall, registration, and runtime management (`servers.scm`,
`registration.scm`) — see `docs/LSP-INSTALL.md`. `core:plum` (the plugin manager) is not
involved.

Requires `core:stdlib` loaded first — diagnostics navigation calls
`stdlib/cursor-char-index` via `call!`.

## Usage

`core:lsp` composes servers you register — either manually via `register-lsp-server!`, or
by installing one with `:lsp-install` (below), which registers it automatically. Add to
your `init.scm`, plus at least one `register-lsp-server!` call if you're not relying on
`:lsp-install`:

```scheme
(load-plugin "core:stdlib")

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(declare-plugin "core:lsp")
```

The zero-argument `declare-plugin` above reads `core:lsp`'s own `manifest.scm`, which
declares the plugin with `#:languages '("*")` (any buffer with a detected language) plus
every `lsp-*` command — so it activates on the first buffer with a language, the first
`lsp-*` command you type, or the first server attach, whichever comes first.
`(load-plugin "core:lsp")` also works if you'd rather load it eagerly.

## Customizing activation

Pass `#:commands`/`#:events`/`#:languages` explicitly to `declare-plugin` to override the
manifest's defaults — e.g. restrict activation to specific languages instead of any:

```scheme
(declare-plugin "core:lsp"
  #:languages '("rust")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"
               "lsp-install" "lsp-uninstall" "lsp-servers" "lsp-rescan-servers"
               "lsp-status" "lsp-stop" "lsp-restart"))
```

Any explicit `#:commands`/`#:events`/`#:languages` bypasses the manifest entirely — the
list above is exactly what `manifest.scm` declares by default, so start from it and trim.

**Caveat**: a manifest keyed only on `#:events '("on-lsp-attach")` can never activate on
its own — nothing is registered yet, so nothing attaches, so the event never fires. Load
`core:lsp` eagerly, add `#:languages` naming the languages you want servers installed
for, or add the four `lsp-*` install commands to `#:commands` as above, so typing one of
them triggers activation directly.

See the [user manual](../../../../user-manual/docs/lsp.md) for the full walkthrough, commands,
keys, and settings.

## Commands

LSP server management:

| Command                | Effect                                                                       |
|-------------------------|-------------------------------------------------------------------------------|
| `:lsp-install [lang]`  | Download, verify, unpack, and register the server for a language (default: current buffer's language) |
| `:lsp-uninstall <name>`| Shut down and unregister a server's clients, remove it from disk (by server name, not language) |
| `:lsp-servers`         | Catalog listing: every seeded server, its languages, and install status      |
| `:lsp-rescan-servers`  | Re-scan `<data>/servers/` and register any installed server not yet registered — useful for a server installed out-of-band |
| `:lsp-status`          | Show every running server and its state, plus attached buffers' diagnostic counts |
| `:lsp-stop [lang]`     | Stop a running server (default: focused buffer's)                            |
| `:lsp-restart [lang]`  | Stop and respawn a running server (default: focused buffer's)                |

## Keys

| Key   | Command                       |
|-------|--------------------------------|
| `g d` | lsp-goto-definition             |
| `g D` | lsp-goto-declaration            |
| `g y` | lsp-goto-type-definition        |
| `g i` | lsp-goto-implementation         |
| `g r` | lsp-references                  |
| `g R` | lsp-rename                      |
| `g k` | lsp-hover                       |
| `g a` | lsp-code-actions                |
| `g n` | goto-next-diagnostic            |
| `g p` | goto-prev-diagnostic            |
| `Ctrl+Space` (Insert) | lsp-completion-trigger |

No collisions with HUME's default `g` goto trie (`g g e h l s`) at the time these were bound —
re-check `keymap/defaults.rs` if you rebind any of the native goto leaves.

`lsp-fmt` and `diagnostics` are typed-command only (`:lsp-fmt`, `:diagnostics`) — no default key.
All keybindings above are bound by this plugin itself — with no `core:lsp` loaded or declared,
none of these keys do anything.

## How it works

`manifest.scm` holds nothing but the default `declare-plugin` call shown above — HUME
evaluates it only when `init.scm` calls `(declare-plugin "core:lsp")` with no explicit
activation entries. One `plugin.scm` entry `require`s a file per feature area (`hover.scm`, `goto.scm`,
`diagnostics.scm`, `rename.scm`, `format.scm`, `actions.scm`, `sighelp.scm`,
`completion.scm`, `inlay.scm`), plus a shared `lib.scm` (capability checks, error
reporting, the viewport tracker, location-drawer helper), `registration.scm` (the seeded
catalog, receipt/path helpers, and the scan), and `servers.scm` (install/uninstall — see
below). Every feature file is the same three-line shape: send an `lsp-request`,
transform the response, call a UI or store builtin.

### Server install and registration

`servers.scm` downloads, verifies, and unpacks a server (`:lsp-install`), writes a
receipt as the install commit point, and calls `registration.scm`'s scan
(`lsp/register-installed-servers!`) directly afterward so the server attaches
immediately — no cross-plugin notify, since install and registration are the same
plugin. That scan independently reads the seeded `runtime/scheme/lsp-servers.scm`
catalog and `<data>/servers/` for receipts, registering (`register-lsp-server!`) every
installed server it finds; `plugin.scm` runs it once at its own top level, so it also
happens at load or lazy activation. `:lsp-rescan-servers` exposes the same scan for a
server installed outside `:lsp-install`.

### Runtime management

`registration.scm` also defines `:lsp-status`/`:lsp-stop`/`:lsp-restart` — thin wrappers
over the `lsp-show-status!`/`lsp-stop!`/`lsp-restart!` Rust builtins, which queue the
actual work for the end-of-eval drain (the Steel-eval-time host doesn't hold `&mut
Editor`, only `Editor::apply_lsp_server_ops` does). These are `core:lsp` commands, not
ambient Rust builtins — a `register-lsp-server!` config with no `core:lsp` loaded has no
way to inspect, stop, or restart the server it spawns.
