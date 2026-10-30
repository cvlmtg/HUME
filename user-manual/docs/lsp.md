# Language Servers

`core:lsp` connects HUME to language servers for hover docs, go-to-definition, references, symbol search, diagnostics, rename, formatting, code actions, signature help, completions and inlay hints. The companion core plugin `core:lsp-install` installs the servers themselves, so getting a language working is usually one command.

## Setup

Load `core:lsp` and `core:lsp-install` from your [`init.scm`](configuration.md) (see [Core Plugins](core-plugins.md#core-lsp) for how they fit alongside HUME's other core plugins):

```scheme
(load-plugin! "core:stdlib")       ; the plugins depend on it
(load-plugin! "core:lsp")
(load-plugin! "core:lsp-install")  ; :lsp-install and friends; leave it out to manage servers yourself
```

Neither plugin loads at startup: each loads the first time a buffer gets a language, or when you run one of its commands. To start `core:lsp` only for some languages, or only from some commands, see [Choosing your own triggers](plugins.md#choosing-your-own-triggers).

A language uses the servers its [server list](#choosing-which-server-a-language-uses) names. Opening a file attaches every server on its language's list, starting each one once per project root. [`:lsp-install`](#installing-servers) installs a server, registers it and puts it on the list, so it is usually all you need. To manage a server yourself, [register it by hand](#registering-a-language-server) instead.

## Installing servers

`core:lsp-install` downloads and manages language servers for you, the same way [PLUM](core-plugins.md#core-plum) handles tree-sitter grammars.

### Prerequisites

Installing a server runs a few external tools. If one is missing, the install names it before downloading anything. Depending on the server:

- `curl`: for servers downloaded as a release asset
- `gzip`: for servers distributed as a single gzip-compressed binary
- `unzip` (macOS/Linux) or `tar` (Windows): for servers distributed as a zip archive
- `tar`: for servers distributed as a tar archive; on Linux, `xz` or `bzip2` too for `.tar.xz` or `.tar.bz2` archives
- `npm`: for servers distributed as an npm package
- `cargo`: for servers built from a Rust crate (compiled from source; the first install can take a few minutes)
- `go`: for servers installed with `go install`
- `python3` (`python` on Windows), with `venv` and `pip`: for servers distributed on PyPI
- `gem`: for servers distributed as a Ruby gem
- `dotnet`: for servers distributed as a .NET tool

Install them with your system's package manager; `cargo` comes from [rustup.rs](https://rustup.rs). On Windows 10 and later, `tar` and `curl` come with the system and `gzip` comes with Git for Windows.

### Install a server

Open a file in the language you want a server for, then run:

```
:lsp-install
```

Or name the language directly. Tab completes language names and the servers `:lsp-install` can install:

```
:lsp-install rust
```

HUME installs the pinned version of the server, then registers it. A server published as a download is fetched and checked against its checksum; a server published as a package is installed through its package manager (npm, cargo, go, pip, gem or dotnet). Already-open buffers of that language attach immediately, with no restart. Running `:lsp-install` again for a server already at the pinned version never re-downloads.

`:lsp-install` with a language installs the first server HUME knows for it. Some languages have more than one (Python has several, for example). To install another, name the server:

```
:lsp-install ruff
```

Naming a server HUME can't install tells you why ([Troubleshooting](#troubleshooting)).

Installed servers for one language run together on a buffer, in the order HUME's catalog lists them, with any limits the catalog puts on what each is used for. To choose the order or the limits yourself, see [Choosing which server a language uses](#choosing-which-server-a-language-uses).

You don't need to run this ahead of time: opening a file whose language has an installable, uninstalled server shows a one-line `run :lsp-install` hint, once per language per session.

### See what's available

```
:lsp-catalog
```

Opens a read-only `[lsp-catalog]` buffer listing every server HUME knows about, by name, with its languages and whether it's installed, out of date, or why it can't be installed here. A language a server only backs second to another is marked `(secondary)`. Many entries are listed so you can point HUME at a copy you install yourself, with [`register-lsp-server!`](#registering-a-language-server).

### Manage installed servers

```
:lsp-uninstall <name>
```

Stops the server, unregisters it, and removes it from disk. Use the server's name from `:lsp-catalog`, not the language name: `:lsp-uninstall rust-analyzer`, not `:lsp-uninstall rust`. Tab completes every server installed on disk, including one HUME's catalog doesn't list.

Reinstalling a server that's running (to pick up an update, say) stops it first. If the reinstall fails, or HUME stops in the middle of it, the installed version stays and starts again.

### Troubleshooting

`:lsp-install` reports why it can't install a server:

| Message | Meaning |
|---------|---------|
| `requires '<tool>' on $PATH, which was not found` | A tool the install runs is missing; see [Prerequisites](#prerequisites) |
| `requires a program that extracts <format> archives, which was not found` | No `tar` or `unzip` for the server's archive; see [Prerequisites](#prerequisites) |
| `unsupported platform` | HUME installs servers on macOS (Apple silicon and Intel), Linux x86-64 and Windows x86-64 only |
| `no prebuilt asset for this platform`, `not supported on this platform` | The server's own releases don't cover your system |
| `no install source` | HUME knows the server but has no way to install it |
| `not installable (kind …)` | The server has to be built from source, or comes from a package manager `:lsp-install` doesn't use (such as opam or luarocks) |

For the last three, install the server yourself and [register it by hand](#registering-a-language-server).

**A server installs but fails to start.** A few servers need more than the install provides: the Java server (jdtls) needs a JDK and `python3` on your `$PATH` when it runs.

**A server is on disk but nothing attaches.** `core:lsp-install` is probably not loaded. Add `(load-plugin! "core:lsp-install")` to your `init.scm`.

**A server on your `$PATH` isn't the one HUME runs.** `:lsp-install` always runs the managed copy, even when the same command name is also on your `$PATH`, and a note after installing says so. To run your `$PATH` copy instead, [register it by hand](#registering-a-language-server) under the same name.

## Registering a language server

Register a server by hand when you don't install it with [`:lsp-install`](#installing-servers): a locally built server, a version HUME's catalog doesn't carry, or a `$PATH` copy you want instead of a managed install. `register-lsp-server!` itself doesn't need `core:lsp`, but the [commands that inspect, stop or restart servers](#managing-servers) come from `core:lsp`.

A registration says how to start a server; a language uses it once the language's [server list](#choosing-which-server-a-language-uses) names it. The project root comes from the language's own root markers (`Cargo.toml` for Rust, `go.mod` for Go, and so on), which the bundled languages already carry.

```scheme
(register-lsp-server! "pyright" #:command "pyright-langserver" #:args '("--stdio"))
(set-language-servers! "python" '("pyright"))
```

`register-lsp-server!` takes:

| Argument | Meaning |
|----------|---------|
| name | The server's name, with no spaces. `:lsp-stop`/`:lsp-restart` take it, and registering the same name again replaces its registration |
| `#:command` | The executable to run |
| `#:args` | Extra command-line arguments, if the server needs them |
| `#:init-options` | Server configuration, sent once at startup; see [Server configuration](#server-configuration-init-options-and-settings) |
| `#:settings` | Server configuration, available to the server while it runs; see [Server configuration](#server-configuration-init-options-and-settings) |
| `#:env` | Extra environment variables for the server process, as a list of `("NAME" . "value")` pairs, added to HUME's own environment |

A server you register under the same name as one installed with `:lsp-install` takes its place, wherever the call sits in your `init.scm`: `core:lsp-install` registers only names nobody has registered. Running `:lsp-install` for that name replaces your registration until the next start, when yours applies again. To keep both, register yours under a name of its own.

### Choosing which server a language uses

A language uses the servers its list names, and a buffer attaches to all of them, in list order. With `core:lsp-install` loaded, every language it knows starts with a list of its own. Any other language has no list until you set one, so a server you register yourself serves nothing until a list names it.

With several servers attached:

- diagnostics from every server show together
- completion lists every server's items
- code actions list every server's actions, with the server's name beside each when several offer some
- goto and references combine every server's locations
- document and workspace symbols list every server's symbols
- inlay hints come from all of them
- hover, signature help, rename and formatting use the first server in the list that is running and supports the feature

Give the names in the order you want:

```scheme
(set-language-servers! "python" '("pyright" "ruff"))
```

A list names the only servers a language uses, so adding a server means restating the whole list. `:lsp-status` shows which languages each running server serves. A name that isn't registered yet takes its place in the list once it registers.

An entry can also limit a server to some features, with `only-features` or `except-features` (give one, not both):

```scheme
(set-language-servers! "python"
  (list "pyright" (hash 'name "ruff" 'only-features '(format diagnostics))))
```

A server limited this way contributes only those features: here, ruff's diagnostics and formatting are used and its other answers are not. The feature names are:

- navigation: `goto-declaration`, `goto-definition`, `goto-type-definition`, `goto-implementation`, `goto-reference`, `document-symbols`, `workspace-symbols`, `call-hierarchy`, `document-links`
- information: `hover`, `signature-help`, `document-highlight`, `inlay-hints`, `document-colors`
- editing: `completion`, `code-action`, `rename-symbol`, `format`, `workspace-command`
- diagnostics: `diagnostics`, `pull-diagnostics`

A list you set, in your `init.scm` or from a command you run, beats one a plugin sets, whichever comes first. Plugins ship their own with `set-default-language-servers!`, which takes the same arguments and applies only while no `set-language-servers!` list is set. Passing `#f` instead of a list clears what you set, and the plugin's list applies again.

### Server configuration (`#:init-options` and `#:settings`)

Each language server has its own configuration options, named and shaped by its own documentation. Check your server's docs for the option names.

`#:init-options` is sent once, when the server starts. Most servers read their configuration this way:

```scheme
(register-lsp-server! "gopls" #:command "gopls"
  #:init-options (hash "usePlaceholders" #t
                       "hints" (hash "parameterNames" #t)))
```

`#:settings` is sent right after the server is ready, and HUME also answers from it when the server later asks for a piece of its configuration by name. A piece your hash doesn't have gets an empty answer. Some servers read only one of the two, so when your server's docs show a settings file, pass the same hash as both.

Changing either and running `:reload-config` updates what HUME has stored, but a running server keeps the configuration it started with until it restarts, and HUME says so once in the message log. Run `:lsp-restart` (or reinstall the server) to apply the change.

## Commands and keys

| Key | Command | Effect |
|-----|---------|--------|
| `K` | `lsp/hover` | Show docs for the symbol under the cursor |
| `g d` | `lsp/goto-definition` | Jump to the symbol's definition |
| `g D` | `lsp/goto-declaration` | Jump to the symbol's declaration |
| `g y` | `lsp/goto-type-definition` | Jump to the definition of the symbol's type |
| `g i` | `lsp/goto-implementation` | Jump to the symbol's implementation |
| `z r` | `lsp/references` | List every reference to the symbol |
| `G R` | `lsp/rename` | Rename the symbol under the cursor everywhere it's used |
| `z a` | `lsp/code-actions` | Show fixes and refactors available at the cursor |
| `z s` | `lsp/document-symbols` | Pick a symbol of this file (functions, types, and so on) and jump to it |
| `z S` | `lsp/workspace-symbols` | Search the symbols of the whole project as you type and jump to one |
| `g n` | `lsp/goto-next-diagnostic` | Jump to the next error or warning after the cursor (wraps) |
| `g p` | `lsp/goto-prev-diagnostic` | Jump to the previous error or warning before the cursor (wraps) |
| — | `:diagnostics` | List every diagnostic in the buffer, each with the tool that reported it |
| — | `:format-source` | Format the selected lines if every selection spans whole lines, the whole buffer if none does, and nothing (with a message) if it's a mix of the two |
| — | `lsp/format-source` | The same as `:format-source`, as an editor command you can bind to a key |
| — | `:toggle-inlay-hints` | Show or hide inlay hints in the current buffer |
| `Ctrl-Space` (Insert) | `completion-trigger` | Show completions at the cursor |

Jumping to a definition, declaration, type, implementation or reference in another file opens that file as a buffer; `Ctrl-o` jumps back. A goto with more than one match opens a list to pick from instead of jumping, and `z r` always opens the list, even for a single hit.

That list (the same one `:diagnostics` opens) is a drawer across the bottom of the screen. `Ctrl-d`/`Ctrl-u` move half the drawer's height, `Down`/`Up` move one row, `Enter` jumps to the highlighted row, and `Esc` closes it. Every other key, including `j`/`k`, reaches the buffer underneath. The drawer stays open while you edit, so you can browse a long references list and edit without losing your place in either.

The list follows your edits. When the number of lines changes in the file you asked from or in a file it lists, or an edit touches or moves a listed line, HUME asks the language server again about the same symbol once you pause typing, and replaces the rows. If nothing is found any more, the drawer closes with a message.

Typing while a completion menu is open narrows it. `Tab` and `Down` move to the next entry, `Shift-Tab` and `Up` to the previous, `Enter` accepts the highlighted one, and `Esc` dismisses the menu. Signature help pops up as you type an argument list for a function the server knows about, and inlay hints appear inline once enabled.

After `g n` or `g p` jumps, a popup shows the full diagnostic message. `Ctrl-u`/`Ctrl-d` scroll it, and any other key, paste or mouse action closes it. Each line with a problem also gets a short summary at its end. While you type in Insert mode, the underline and summary are hidden on the cursor's line, and return when you leave Insert mode or move to another line. Set `lsp-diagnostics-on-insert-line` to `#t` to keep them showing.

A server that reports diagnostics only when asked is asked when the file opens, after its text changes, and when you save.

## Settings

The `lsp-*` options configure HUME's language-server support, shared by every plugin that talks to a language server. They are global options, described in [Global options](settings.md#global-options), and you set them like any other:

```scheme
(set-option! "lsp-diagnostics-min-severity" "warning")
```

`core:lsp`'s own options are passed with `#:config` when you load it:

```scheme
(load-plugin! "core:lsp" #:config (hash 'inlay-hints #t 'format-max-ranges 32))
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `inlay-hints` | bool | `#f` | Show inferred types and parameter names inline, next to the code they describe, in every buffer. `:toggle-inlay-hints` turns them on or off for the current buffer |
| `format-max-ranges` | integer ≥ 1 | `16` | Above this many separate ranges, `:format-source` formats nothing and says so |

### Format on save

Off by default. To format the whole buffer every time you save, before the file is written, add this to your `init.scm`:

```scheme
(register-hook! 'on-before-buffer-save
  (lambda (pane done) (call! "lsp/format-document" pane done)))
```

The save waits for the language server's answer, and `:wa` also formats buffers that no pane is showing. If there is no server, or it fails or doesn't answer within `hook-before-save-timeout-ms` (see [Global options](settings.md#global-options)), the buffer is saved as it is. An answer that arrives later still formats the buffer, leaving it modified.

`lsp/format-document` also takes an optional third argument: a list of LSP ranges to format instead of the whole buffer, such as the `"ranges"` list [`lsp-linewise-ranges-params`](lsp-api.md#language-servers) returns.

## Managing servers

Commands for a server that's already running. To install, browse the catalog, or remove a server from disk, see [Installing servers](#installing-servers).

| Command | Effect |
|---------|--------|
| `:lsp-status` | Show every running server and its state, each buffer's servers, and the servers you stopped |
| `:lsp-stop [name]` | Stop a server (default: every server on the focused buffer); with a name, every running copy of that server. It stays stopped, even for new buffers and server-list changes, until `:lsp-restart`, `:reload-config` or a new `register-lsp-server!` of its name |
| `:lsp-restart [name]` | Stop and restart a server, or start one that was stopped (default: every server on the focused buffer) |

A server stops by itself once no open buffer uses it: after you close its last buffer, or when a server list, or a buffer's language, no longer includes it. Opening a matching file starts it again. A server that crashed stays down for its buffers until you run `:lsp-restart`.

Server output and protocol errors show in `:messages`.

A plugin can also send a server requests of its own, such as a server extension `core:lsp` doesn't use. See [Sending a custom request](lsp-api.md#sending-a-custom-request).
