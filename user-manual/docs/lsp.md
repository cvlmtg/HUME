# Language Servers

Everything an IDE gives you, in the terminal. `core:lsp` connects HUME to language servers
for hover docs, go-to-definition, references, diagnostics, rename, formatting, code actions,
signature help, completions, and inlay hints — and installs the servers themselves, so
getting a language working is usually one command.

## Setup

Bring in `core:lsp` from your `init.scm`, and make sure a server is registered for the
languages you use. The easiest way to get a server is
[`:lsp-install`](#installing-servers) — run it once per language and it downloads,
verifies, and registers the server in one step, no separate download tool needed. If
you'd rather manage a server yourself (a local build, a version the seeded catalog
doesn't carry, or a `$PATH` copy you want to take precedence), register it by hand
instead — see [Registering a language server](#registering-a-language-server).

```scheme
(load-plugin "core:stdlib")      ; core:lsp depends on it, and needs it loaded, not just declared
(declare-plugin "core:lsp")
```

Declaring `core:lsp` is recommended — it keeps startup fast, and `core:lsp` activates the
first time any file with a recognized language opens, or you run one of its commands
directly (including `:lsp-install`). If you use LSP in every session and would rather it
load from the start, swap `declare-plugin` for `load-plugin`:

```scheme
(load-plugin "core:lsp")
```

Want activation to only trigger for specific languages, or a smaller set of commands?
Pass `#:languages`/`#:commands`/`#:events` to `declare-plugin` yourself and it uses
exactly what you list instead of the defaults:

```scheme
(declare-plugin "core:lsp"
  #:languages '("rust")   ; only activate for languages you name here
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"
               "lsp-install" "lsp-uninstall" "lsp-servers" "lsp-rescan-servers"
               "lsp-status" "lsp-stop" "lsp-restart"))
```

::: warning
`#:events '(on-lsp-attach)` by itself never activates on its own — nothing
is registered yet, so nothing attaches, so the event that would trigger activation never
fires. List the languages you want servers for in `#:languages`, list the `lsp-*`
commands in `#:commands` (as above), or load `core:lsp` eagerly — any one of these gets
you a working `:lsp-install`.
:::

Opening a file whose language matches a registered server spawns it automatically (once per
project root) and attaches.

## Installing servers

`core:lsp` downloads and manages language servers for you, the same way [PLUM](core-plugins.md#core-plum)
handles tree-sitter grammars — no need to track down a binary or install it by hand.

### Prerequisites

Installing a server shells out to a few external tools. Most are already on your system; if
one is missing, the install tells you which one before downloading anything. Depending on the
server:

- `curl` — for servers downloaded as a release asset
- `gzip` — for servers distributed as a single gzip-compressed binary
- `unzip` (macOS/Linux) or `tar` (Windows) — for servers distributed as a zip archive
- `npm` — for servers distributed as an npm package
- `cargo` — for servers built from a Rust crate (compiled from source; the first install
  can take a few minutes)

How you install these depends on your operating system:

- **macOS**: [Homebrew](https://brew.sh) — `brew install curl gzip unzip node`; install
  `cargo` via [rustup.rs](https://rustup.rs).
- **Linux**: use your distribution's package manager; these are usually already installed
  except `node`/`npm`, which most distros package as `nodejs`/`npm`, and `cargo`, best
  installed via [rustup.rs](https://rustup.rs) rather than a distro package.
- **Windows**: `tar` and `curl` ship with Windows 10+; `gzip` needs Git for Windows (or an
  equivalent) on `PATH`; install `node` from [nodejs.org](https://nodejs.org) or via
  [winget](https://learn.microsoft.com/windows/package-manager/winget/)/[Scoop](https://scoop.sh);
  install `cargo` via [rustup.rs](https://rustup.rs).

### Install a server

Open a file in the language you want a server for, then run:

```
:lsp-install
```

Or name the language directly:

```
:lsp-install rust
```

HUME downloads the pinned release, verifies its checksum, unpacks it, and registers it —
already-open buffers of that language attach immediately, no restart needed. Running
`:lsp-install` again for a server that's already at the latest seeded version never
re-downloads, but still re-registers — useful if a receipt was installed out-of-band and
hasn't attached yet. (A language you've registered by hand is left alone by any
rescan, whether or not the server is also managed by `:lsp-install`.)

You don't need to run this ahead of time: opening a file whose language has an installable,
uninstalled server shows a one-line `run :lsp-install` hint, once per language per session.

### See what's available

```
:lsp-servers
```

Lists every server HUME knows about: its languages, its seeded version, and whether it's
installed, out of date, or not installable on your platform (and why). Not every entry can
be installed for you — many are listed so you can point HUME at a copy you install yourself,
with `register-lsp-server!` below.

### Manage installed servers

```
:lsp-uninstall <name>
```

Shuts down any running client for that server, unregisters it, and removes it from disk. Use
the server's name from `:lsp-servers`, not the language name — e.g.
`:lsp-uninstall rust-analyzer`, not `:lsp-uninstall rust`.

Reinstalling a server that's already running (e.g. to pick up an update) shuts the old client
down first. If the install fails anyway — a locked file on Windows is the usual reason — the
message tells you to run `:lsp-install` again, which normally succeeds the second time.

### Troubleshooting

**`:lsp-install` fails naming a missing tool.** Install it — see the
[prerequisites](#prerequisites) above.

**`:lsp-install` says "not installable".** Not every server HUME knows about can be
auto-installed — some don't publish prebuilt binaries HUME can unpack, are pinned by their
package manager to a git revision rather than a released version, or are only available
through a package manager not yet supported (`pip`, `gem`, …). Install it yourself
and register it manually — see [Registering a language server](#registering-a-language-server)
below.

**A server is on disk but nothing attaches.** This means `core:lsp`'s scan hasn't (yet)
seen it — either `core:lsp` hasn't loaded or activated this session at all (a lazily
declared `core:lsp` whose trigger hasn't fired yet), or the server appeared on disk after
the scan already ran (installed outside `:lsp-install` — copied in, or installed by an
earlier HUME version). Run `:lsp-rescan-servers`, add `(load-plugin "core:lsp")`, or add a
`#:languages`/`#:commands` entry that triggers activation on a lazily declared `core:lsp`
— see [Setup](#setup).

**A server on your `$PATH` isn't the one HUME runs.** `:lsp-install` always spawns the managed
copy, even when the same command name also resolves on `$PATH` — you'll see a note about this
after installing. Register the server manually instead if you want your `$PATH` copy to take
precedence.

## Registering a language server

Registering by hand is only needed if you're not using
[`:lsp-install`](#installing-servers) — a locally built server, a version the seeded
catalog doesn't carry, or a `$PATH` copy you want to take precedence over a managed install.
`register-lsp-server!` itself has no dependency on `core:lsp` — but the
[managing-servers commands](#managing-servers) below (`:lsp-status`, `:lsp-stop`,
`:lsp-restart`) are `core:lsp` commands, so a manually registered server still needs
`core:lsp` loaded or declared to inspect, stop, or restart it. A manual `register-lsp-server!`
call always overrides a seeded, installed server for the same language, whether it comes
before or after `(load-plugin "core:lsp")` or `(declare-plugin "core:lsp")` in your init.scm —
order doesn't matter. `register-lsp-server!` takes:

| Argument | Meaning |
|----------|---------|
| language | A name you choose (matches HUME's own language identity for a buffer — `"rust"`, `"python"`, `"typescript"`, …) |
| `#:command` | The executable to run |
| `#:args` | Extra command-line arguments, if the server needs them |
| `#:root-markers` | Filenames that mark a project root (HUME walks up from the opened file looking for the nearest one) |
| `#:init-options` | Server configuration, sent once at startup — see below |
| `#:settings` | Server configuration, handed over on an ongoing basis — see below |
| `#:env` | Extra environment variables for the server process, as a list of `("NAME" . "value")` pairs — added on top of the environment HUME itself runs in, never replacing it |

Examples for a few commonly used servers:

```scheme
;; Rust — rust-analyzer
(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

;; Python — pyright
(register-lsp-server! "python" #:command "pyright-langserver" #:args '("--stdio")
                               #:root-markers '("pyproject.toml" "setup.py"))

;; TypeScript / JavaScript — typescript-language-server
(register-lsp-server! "typescript" #:command "typescript-language-server" #:args '("--stdio")
                                   #:root-markers '("package.json" "tsconfig.json"))

;; Go — gopls
(register-lsp-server! "go" #:command "gopls" #:root-markers '("go.mod"))

;; C / C++ — clangd
(register-lsp-server! "c" #:command "clangd" #:root-markers '("compile_commands.json" ".clangd"))
```

### Server configuration (`#:init-options` and `#:settings`)

Language servers each have their own configuration options — code style, extra warnings, where to find things — and most of them read the *shape and names* of those options straight from their own documentation, not from anything LSP-specific. HUME just needs to hand the hash over in the right way, and servers differ on which way that is. Check your server's own docs for the option names, and try `#:init-options` first.

**`#:init-options` is sent once, when the server starts up.** This is how most servers actually pick up their configuration — including well-known ones like rust-analyzer and gopls:

```scheme
;; gopls reads its own option names directly off the hash you pass here —
;; no extra nesting needed.
(register-lsp-server! "go" #:command "gopls"
  #:root-markers '("go.mod")
  #:init-options (hash "hints" (hash "assignVariableTypes" #t
                                     "parameterNames" #t)
                       "usePlaceholders" #t))
```

**`#:settings` is handed to the server on an ongoing basis instead of just once at startup:** HUME sends it right after the server is ready, and keeps it on hand to answer if the server asks for a specific piece of it later by name. Which of those two a given server actually pays attention to depends on the server — some read what's handed to them upfront, some ask for a named piece, some do both — so when in doubt, shape the hash to match whatever your server's own docs show for a settings file, and pass it as both:

```scheme
;; typescript-language-server reads its "typescript" and "javascript"
;; keys from what's handed to it, without asking for them by name.
(register-lsp-server! "typescript" #:command "typescript-language-server" #:args '("--stdio")
  #:root-markers '("package.json" "tsconfig.json")
  #:settings (hash "typescript" (hash "inlayHints" (hash "parameterNames" (hash "enabled" "all")))))
```

If a server does ask for a specific named piece and it isn't in your `#:settings` hash, that's a plain "nothing configured for this" answer, not an error — well-behaved servers, including gopls and rust-analyzer, take that in stride and keep whatever configuration they already have.

Changing either and running `:reload-config` updates what HUME has stored, but an already-running server keeps its old configuration until it restarts — run `:lsp-restart` (or reinstall the server) to push the change.

## Commands and keys

| Key   | Command                    | Effect |
|-------|------------------------------|--------|
| `z k` | `lsp-hover`                  | Show docs for the symbol under the cursor |
| `g d` | `lsp-goto-definition`        | Jump to the symbol's definition |
| `g D` | `lsp-goto-declaration`       | Jump to the symbol's declaration |
| `g y` | `lsp-goto-type-definition`   | Jump to the symbol's type's definition |
| `g i` | `lsp-goto-implementation`    | Jump to the symbol's implementation |
| `z r` | `lsp-references`             | List every reference to the symbol |
| `g r` | `lsp-rename`                 | Rename the symbol under the cursor everywhere it's used |
| `z a` | `lsp-code-actions`           | Show fixes and refactors available at the cursor |
| `g n` | `goto-next-diagnostic`       | Jump to the next error/warning after the cursor (wraps) |
| `g p` | `goto-prev-diagnostic`       | Jump to the previous error/warning before the cursor (wraps) |
| —     | `:diagnostics`               | List every diagnostic in the buffer |
| —     | `:lsp-fmt`                   | Format the buffer, or just the selected lines if the selection spans whole lines |
| `Ctrl+Space` (Insert) | `lsp-completion-trigger` | Show completions at the cursor |

Jumping to a definition, declaration, type, implementation, or reference in another file
opens that file as a buffer; `Ctrl+o` jumps back. A goto with more than one match opens a
list to pick from instead of jumping directly, and `z r` (references) always opens the list,
even for a single hit.

Typing while a completion menu is open narrows it. `Tab` and `Down` move to the next entry,
`Shift+Tab` and `Up` to the previous, `Enter` accepts the highlighted one, and `Esc`
dismisses the menu. Signature help pops up automatically as you type an argument list for a
function the server knows about, and inlay hints (see below) appear inline once enabled.

`g n` and `g p` show the full diagnostic message in a popup after they jump; it clears on
your next keypress or mouse action. Each line with a problem also gets a short summary at
its end.

## Settings

| Setting | Default | Effect |
|---------|---------|--------|
| `lsp.inlay-hints` | `false` | Show inferred types and parameter names inline, next to the code they describe |
| `lsp.request-timeout-ms` | `10000` | How long to wait for a server response before giving up |
| `lsp.viewport-debounce-ms` | `150` | How long to wait after scrolling settles before refreshing viewport-driven features (like inlay hints) |
| `lsp.diagnostics-severity-floor` | `hint` | The lowest diagnostic severity shown (`error`, `warning`, `info`, or `hint`) |

```scheme
(set-option! "lsp.inlay-hints" #t)
```

### Format on save

Not on by default. Add this to your `init.scm` to run `:lsp-fmt` every time you save:

```scheme
(register-hook! 'on-buffer-save
  (lambda (bid) (call! "lsp-fmt")))
```

## Managing servers

Commands for a server that's already running. To install, browse the catalog, or remove a
server from disk, see [Installing servers](#installing-servers).

| Command | Effect |
|---------|--------|
| `:lsp-status` | Show every running server and its state |
| `:lsp-stop [language]` | Stop a server (default: the focused buffer's) |
| `:lsp-restart [language]` | Stop and respawn a server |

Server output and protocol errors are visible in `:messages`.

## Advanced: custom requests

`lsp-request` isn't limited to the built-in commands above — any plugin can call it to reach
a server extension the built-in feature set doesn't cover. This is how you'd add a command
for rust-analyzer's `rust-analyzer/expandMacro`, which expands the macro under the cursor and
returns its generated code:

```scheme
(define-command! "rust-expand-macro" "Show the expansion of the macro under the cursor."
  (lambda ()
    (lsp-request #f "rust-analyzer/expandMacro" (lsp-position-params (current-buffer))
      (lambda (err res)
        (cond
          (err (log! 'error (string-append "expand macro: "
                                           (if (string? err) err (hash-ref err "message")))))
          ((not res) (log! 'info "Not inside a macro"))
          (else (show-popup! (hash-ref res "expansion"))))))))
```

The shape is always the same three steps: send a request built from `lsp-position-params` or
`lsp-range-params`, transform the server's response, and hand the result to a UI or store
builtin (`show-popup!`, `show-menu!`, `show-drawer-list!`, `apply-text-edits!`,
`apply-workspace-edit!`, …). `err` and `res` are never both set — check `err` first and stop
on it, the way every built-in feature does.

`lsp-request` also takes two keyword args for requests that fire more than once. `#:supersede "<key>"` cancels the caller's own previous still-pending request filed under the same key — the server gets `$/cancelRequest` and the old callback never fires — which is how completion's per-keystroke refilter avoids piling up stale requests as you type. `#:allow-stale #t` lets the callback run even if the buffer has changed since the request was sent, for requests where a slightly-out-of-date answer is still useful.

A server's response sometimes carries its own position or range rather than the one you sent — a related location returned inside `res`, say. Convert it back into a plain buffer offset with `lsp-position->offset`/`lsp-range->offsets` before using it with any editing command; both return `#f` if the buffer has no server attached to convert against.
