# Language Servers

`core:lsp` connects HUME to language servers: hover docs, go-to-definition, references,
diagnostics, rename, formatting, code actions, signature help, completions, and inlay hints.

## Setup

Bring in `core:lsp` from your `init.scm`, and make sure a server is registered for the
languages you use. The easiest way to get a server is
[`:lsp-install`](#installing-servers) — run it once per language and it downloads,
verifies, and registers the server in one step, no separate download tool needed. If
you'd rather manage a server yourself (a local build, a version the seeded catalog
doesn't carry, or a `$PATH` copy you want to take precedence), register it by hand
instead — see [Registering a language server](#registering-a-language-server).

```scheme
(load-plugin "core:stdlib")   ; core:lsp depends on it

(declare-plugin "core:lsp"
  #:events '("on-lsp-attach")
  #:languages '("rust")   ; languages you want a server installed/attached for
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"
               "lsp-install" "lsp-uninstall" "lsp-servers" "lsp-rescan-servers"))
```

Declaring is recommended — it keeps startup fast, and `core:lsp` activates the first time a
registered server attaches to a buffer, a matching language opens, or you run one of its
commands directly (including `:lsp-install` itself, if it's listed in `#:commands` as
above). If you use LSP in every session and would rather it load from the start, swap
`declare-plugin` for `load-plugin`:

```scheme
(load-plugin "core:lsp")
```

**Caveat**: `#:events '("on-lsp-attach")` by itself never activates on its own — nothing
is registered yet, so nothing attaches, so the event that would trigger activation never
fires. List the languages you want servers for in `#:languages`, list the `lsp-*`
commands in `#:commands` (as above), or load `core:lsp` eagerly — any one of these gets
you a working `:lsp-install`.

Opening a file whose language matches a registered server spawns it automatically (once per
project root) and attaches.

## Installing servers

`core:lsp` downloads and manages language servers for you, the same way [PLUM](core-plugins.md#plum)
handles tree-sitter grammars — no need to track down a binary or install it by hand.

### Prerequisites

Installing a server shells out to a few external tools. Most are already on your system; if
one is missing, the install tells you which one before downloading anything. Depending on the
server:

- `curl` — always, to download the release asset
- `gzip` — for servers distributed as a single gzip-compressed binary
- `unzip` (macOS/Linux) or `tar` (Windows) — for servers distributed as a zip archive
- `node` and `npm` — for servers distributed as an npm package

How you install these depends on your operating system:

- **macOS**: [Homebrew](https://brew.sh) — `brew install curl gzip unzip node`.
- **Linux**: use your distribution's package manager; these are usually already installed
  except `node`/`npm`, which most distros package as `nodejs`/`npm`.
- **Windows**: `tar` and `curl` ship with Windows 10+; `gzip` needs Git for Windows (or an
  equivalent) on `PATH`; install `node` from [nodejs.org](https://nodejs.org) or via
  [winget](https://learn.microsoft.com/windows/package-manager/winget/)/[Scoop](https://scoop.sh).

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

Lists every server HUME knows how to install: its languages, its seeded version, and whether
it's installed, out of date, or not installable on your platform (and why).

### Manage installed servers

```
:lsp-uninstall <name>
```

Shuts down any running client for that server, unregisters it, and removes it from disk. Use
the server's name from `:lsp-servers`, not the language name — e.g.
`:lsp-uninstall rust-analyzer`, not `:lsp-uninstall rust`.

Reinstalling a server that's already running (e.g. to pick up an update) shuts the old client
down first; on most platforms this completes in one step. If it doesn't (a locked file on
Windows), the message says so — run `:lsp-install` again.

### Troubleshooting

**`:lsp-install` fails naming a missing tool.** Install it — see the
[prerequisites](#prerequisites) above.

**`:lsp-install` says "not installable".** Not every server HUME knows about can be
auto-installed — some don't publish prebuilt binaries HUME can unpack, or are only available
through a package manager not yet supported (`cargo`, `pip`, `gem`, …). Install it yourself
and register it manually — see [Registering a language server](#registering-a-language-server)
below.

**A server is on disk but nothing attaches.** This means `core:lsp` hasn't run its scan
this session yet — either the server was installed outside `:lsp-install` (copied in, or
installed by an earlier HUME version) and `core:lsp` hasn't loaded or activated at all.
Run `:lsp-rescan-servers`, add `(load-plugin "core:lsp")`, or add a `#:languages`/`#:commands`
entry that triggers activation on a lazily declared `core:lsp` — see [Setup](#setup).

**A server on your `$PATH` isn't the one HUME runs.** `:lsp-install` always spawns the managed
copy, even when the same command name also resolves on `$PATH` — you'll see a note about this
after installing. Register the server manually instead if you want your `$PATH` copy to take
precedence.

## Registering a language server

Registering by hand is only needed if you're not using
[`:lsp-install`](#installing-servers) — a locally built server, a version the seeded
catalog doesn't carry, or a `$PATH` copy you want to take precedence over a managed install.
`register-lsp-server!` takes:

| Argument | Meaning |
|----------|---------|
| language | A name you choose (matches HUME's own language identity for a buffer — `"rust"`, `"python"`, `"typescript"`, …) |
| `#:command` | The executable to run |
| `#:args` | Extra command-line arguments, if the server needs them |
| `#:root-markers` | Filenames that mark a project root (HUME walks up from the opened file looking for the nearest one) |
| `#:init-options` | Server-specific initialization options, as a `hash` |
| `#:settings` | Server-specific configuration, as a `hash` — sent once at startup and answered verbatim to the server's own configuration requests |

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

## Commands and keys

| Key   | Command                    | Effect |
|-------|------------------------------|--------|
| `g k` | `lsp-hover`                  | Show docs for the symbol under the cursor |
| `g d` | `lsp-goto-definition`        | Jump to the symbol's definition |
| `g D` | `lsp-goto-declaration`       | Jump to the symbol's declaration |
| `g y` | `lsp-goto-type-definition`   | Jump to the symbol's type's definition |
| `g i` | `lsp-goto-implementation`    | Jump to the symbol's implementation |
| `g r` | `lsp-references`             | List every reference to the symbol |
| `g R` | `lsp-rename`                 | Rename the symbol under the cursor everywhere it's used |
| `g a` | `lsp-code-actions`           | Show fixes and refactors available at the cursor |
| `g n` | `goto-next-diagnostic`       | Jump to the next error/warning after the cursor (wraps) |
| `g p` | `goto-prev-diagnostic`       | Jump to the previous error/warning before the cursor (wraps) |
| —     | `:diagnostics`               | List every diagnostic in the buffer |
| —     | `:lsp-fmt`                   | Format the buffer, or just the selected lines if the selection spans whole lines |
| `Ctrl+Space` (Insert) | `lsp-completion-trigger` | Show completions at the cursor |

Jumping to a definition, declaration, type, implementation, or reference in another file
opens that file as a buffer; use HUME's jump-back binding to return. A goto/references
result with more than one match opens a list to pick from instead of jumping directly.

Typing while a completion menu is open narrows it; `Enter` accepts the highlighted entry,
`Esc` dismisses the menu. Signature help pops up automatically as you type an argument
list for a function the server knows about, and inlay hints (see below) appear inline once
enabled.

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
