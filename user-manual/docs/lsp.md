# Language Servers

Everything an IDE gives you, in the terminal. `core:lsp` connects HUME to language servers
for hover docs, go-to-definition, references, symbol search, diagnostics, rename, formatting, code actions,
signature help, completions, and inlay hints. The companion plugin `core:lsp-install`
installs the servers themselves, so getting a language working is usually one command.

## Setup

Load `core:lsp` and `core:lsp-install` from your [`init.scm`](configuration.md) (see
[Core Plugins](core-plugins.md#core-lsp) for how they fit alongside HUME's other bundled
plugins). Each language you use also needs a server that is registered and named in the
language's list. [`:lsp-install`](#installing-servers) does both: run it once per language
and it downloads, verifies, and registers the server, with no separate download tool. To
manage a server yourself, [register it by hand](#registering-a-language-server) instead.

```scheme
(load-plugin! "core:stdlib")       ; the plugins depend on it
(load-plugin! "core:lsp")
(load-plugin! "core:lsp-install")  ; :lsp-install and friends; leave it out to manage servers yourself
```

Neither plugin slows down startup: each starts the first time you open a file in a language it knows, or run one of its commands.

To start `core:lsp` only for some languages, or only from some commands, see [Choosing your own triggers](plugins.md#choosing-your-own-triggers).

Opening a file attaches every server its language's list names, starting each one once per
project root. `core:lsp-install` fills in a language's [list](#choosing-which-server-a-language-uses)
once it is loaded; otherwise the list is empty until you set one.

## Installing servers

`core:lsp-install` downloads and manages language servers for you, the same way [PLUM](core-plugins.md#core-plum)
handles tree-sitter grammars, so you never hunt down and install a binary by hand.

### Prerequisites

Installing a server runs a few external tools. Most are already on your system; if one is
missing, the install names it before downloading anything. Depending on the server:

- `curl`: for servers downloaded as a release asset
- `gzip`: for servers distributed as a single gzip-compressed binary
- `unzip` (macOS/Linux) or `tar` (Windows): for servers distributed as a zip archive
- `tar`: for servers distributed as a tar archive; on Linux, `xz` or `bzip2` too for
  `.tar.xz` or `.tar.bz2` archives
- `npm`: for servers distributed as an npm package
- `cargo`: for servers built from a Rust crate (compiled from source; the first install
  can take a few minutes)
- `go`: for servers installed with `go install` (gopls)
- `python3` (`python` on Windows), with `venv` and `pip`: for servers distributed on PyPI
  (ty, pylsp)
- `gem`: for servers distributed as a Ruby gem (ruby-lsp)
- `dotnet`: for servers distributed as a .NET tool (the C# and F# servers)

How you install these depends on your operating system:

- **macOS**: [Homebrew](https://brew.sh): `brew install curl gzip unzip node go`; install
  `cargo` via [rustup.rs](https://rustup.rs).
- **Linux**: use your distribution's package manager. These are usually installed already,
  except `node`/`npm` (most distros package them as `nodejs`/`npm`) and `cargo`, best
  installed via [rustup.rs](https://rustup.rs) rather than a distro package.
- **Windows**: `tar` and `curl` ship with Windows 10+. `gzip` needs Git for Windows (or an
  equivalent) on `PATH`. Install `node` from [nodejs.org](https://nodejs.org) or via
  [winget](https://learn.microsoft.com/windows/package-manager/winget/)/[Scoop](https://scoop.sh),
  and `cargo` via [rustup.rs](https://rustup.rs).

### Install a server

Open a file in the language you want a server for, then run:

```
:lsp-install
```

Or name the language directly. Tab completes every language HUME's catalog has a server for, and every installable server:

```
:lsp-install rust
```

HUME downloads the pinned release, verifies its checksum, unpacks it, and registers it.
Already-open buffers of that language attach immediately, with no restart. Running
`:lsp-install` again for a server already at the pinned version never re-downloads. A
download or build replaces a server you've registered by hand under the same name until the
next start, when your own registration applies again. To keep both, register yours under a
name of its own.

`:lsp-install` installs the first server HUME knows for a language. Some languages have
more than one (Python: ty, ruff, jedi, pylsp, zuban; TOML: taplo, tombi). To install
another, name it. Tab completes every server in HUME's catalog; one HUME can't install says why when you run it:

```
:lsp-install ruff
```

Installed servers for one language run together on a buffer, in the order HUME's catalog
lists them, with any limits the catalog puts on what each is used for. To choose the order
or the limits yourself, see
[Choosing which server a language uses](#choosing-which-server-a-language-uses).

You don't need to run this ahead of time: opening a file whose language has an installable,
uninstalled server shows a one-line `run :lsp-install` hint, once per language per session.

### See what's available

```
:lsp-catalog
```

Opens a read-only `[lsp-catalog]` buffer listing every server HUME knows about, by name,
with its languages and whether it's installed, out of date, or not installable on your
platform (and why). A language a server only backs second to another is marked
`(secondary)`. HUME can't install every entry for you: many are listed so you can point HUME
at a copy you install yourself, with `register-lsp-server!` below.

### Manage installed servers

```
:lsp-uninstall <name>
```

Shuts down any running client for that server, unregisters it, and removes it from disk. Use
the server's name from `:lsp-catalog`, not the language name: `:lsp-uninstall rust-analyzer`,
not `:lsp-uninstall rust`. Tab completes every server with an install directory on disk,
including an orphan that HUME's catalog doesn't list.

Reinstalling a server that's already running (e.g. to pick up an update) shuts the old client
down first. If the reinstall fails, or HUME stops in the middle of it, the installed version
stays and starts again.

### Troubleshooting

**`:lsp-install` fails naming a missing tool.** Install it; see the
[prerequisites](#prerequisites) above.

**`:lsp-install` says "not supported on this platform".** The server's own packaging
doesn't support your operating system.

**A server installs but fails to start.** A few servers need more than the install provides:
the Java server (jdtls) needs a JDK 21 or newer and `python3` on your `$PATH` when it runs.

**`:lsp-install` says "not installable".** HUME can't auto-install every server it knows
about. Some don't publish prebuilt binaries HUME can unpack, some are pinned by their package
manager to a git revision rather than a released version, and some are only available through
a package manager not yet supported (`opam`, `luarocks`, …). Install it yourself and register
it as described in [Registering a language server](#registering-a-language-server) below.

**A server is on disk but nothing attaches.** `core:lsp-install` is probably not loaded. Add `(load-plugin! "core:lsp-install")` to your `init.scm`.

**A server on your `$PATH` isn't the one HUME runs.** `:lsp-install` always spawns the managed
copy, even when the same command name also resolves on `$PATH`, and a note after installing
says so. To make your `$PATH` copy take precedence, register it by hand under the same name
`:lsp-catalog` shows for the managed server (`"rust-analyzer"`, not `"rust"`), as described in
[Registering a language server](#registering-a-language-server).

## Registering a language server

You only need to register a server by hand if you're not using
[`:lsp-install`](#installing-servers) for it: a locally built server, a version HUME's
catalog doesn't carry, or a `$PATH` copy you want to take precedence over a managed install.
`register-lsp-server!` itself doesn't need `core:lsp`, but inspecting, stopping or restarting
the server does: the [managing-servers commands](#managing-servers) (`:lsp-status`,
`:lsp-stop`, `:lsp-restart`) come from `core:lsp`. A `register-lsp-server!` call under the
same name as a server installed with `:lsp-install` replaces it, whether it comes before or
after `(load-plugin! "core:lsp")` in your init.scm.

A registration says how to start a server. A server serves a language only once that
language's server list names it, so each example below also adds the server to its
languages' lists with [`set-language-servers!`](#choosing-which-server-a-language-uses). The
project root comes from the language's own root markers, which the bundled languages already
carry (`Cargo.toml` for Rust, `go.mod` for Go, and so on). `register-lsp-server!` takes:

| Argument | Meaning |
|----------|---------|
| name | The server's name, with no spaces. `:lsp-stop`/`:lsp-restart` take it, and registering the same name again replaces its registration |
| `#:command` | The executable to run |
| `#:args` | Extra command-line arguments, if the server needs them |
| `#:init-options` | Server configuration, sent once at startup; see below |
| `#:settings` | Server configuration, handed over on an ongoing basis; see below |
| `#:env` | Extra environment variables for the server process, as a list of `("NAME" . "value")` pairs, added on top of HUME's own environment rather than replacing it |

Examples for a few commonly used servers:

```scheme
;; Rust — rust-analyzer
(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(set-language-servers! "rust" '("rust-analyzer"))

;; Python — pyright
(register-lsp-server! "pyright" #:command "pyright-langserver" #:args '("--stdio"))
(set-language-servers! "python" '("pyright"))

;; TypeScript / JavaScript — typescript-language-server
(register-lsp-server! "typescript-language-server"
                      #:command "typescript-language-server" #:args '("--stdio"))
(set-language-servers! "typescript" '("typescript-language-server"))
(set-language-servers! "tsx" '("typescript-language-server"))

;; Go — gopls
(register-lsp-server! "gopls" #:command "gopls")
(set-language-servers! "go" '("gopls"))

;; C / C++ — clangd
(register-lsp-server! "clangd" #:command "clangd")
(set-language-servers! "c" '("clangd"))
(set-language-servers! "cpp" '("clangd"))
```

### Choosing which server a language uses

A language uses the servers its list names, and a buffer attaches to all of them, in list
order. With `core:lsp-install` loaded, every language it knows starts with a list of its
own. Any other language has no list until you set one, so a server you register yourself
serves nothing until a list names it.

With several servers attached:

- diagnostics from every server show together
- completion lists every server's items
- code actions list every server's actions, with the server's name beside each when several
  offer some
- goto and references combine every server's locations
- inlay hints come from all of them
- hover, signature help, rename and formatting use the first server in the list that is
  running and supports the feature

Give the names in the order you want:

```scheme
(set-language-servers! "python" '("pyright" "ruff"))
```

A list names the only servers a language uses, so adding a server means restating the whole
list.

::: tip ESLint beside typescript-language-server
Install the ESLint server once:

```
:lsp-install vscode-eslint-language-server
```

Then list it after typescript-language-server for JavaScript and TypeScript in your `init.scm`:

```scheme
(for-each
  (lambda (language)
    (set-language-servers! language
      '("typescript-language-server" "vscode-eslint-language-server")))
  '("javascript" "jsx" "typescript" "tsx"))
```

ESLint reports only when asked, so HUME asks when the file opens, after you stop typing, and
when you save.
:::

`:lsp-status` shows which languages each running server serves. A name that isn't registered
yet takes its place in the list once it registers.

An entry can also limit a server to some features, with `only-features` or `except-features`
(give one, not both), using these feature names: `format`, `goto-declaration`, `goto-definition`, `goto-type-definition`, `goto-reference`, `goto-implementation`, `signature-help`, `hover`, `document-highlight`, `completion`, `code-action`, `document-links`, `workspace-command`, `document-symbols`, `workspace-symbols`, `diagnostics`, `pull-diagnostics`, `rename-symbol`, `inlay-hints`, `document-colors`, `call-hierarchy`.

```scheme
(set-language-servers! "python"
  (list "pyright" (hash 'name "ruff" 'only-features '(format diagnostics))))
```

A server limited this way contributes only those features: with `'(format diagnostics)`,
ruff's diagnostics are shown and its other answers are not used.

A list you set, in your `init.scm` or from a command you run, beats one a plugin sets,
whichever comes first. Plugins ship their own with `set-default-language-servers!`, which
takes the same arguments and applies only while no `set-language-servers!` list is set.
Passing `#f` instead of a list clears what you set, and the plugin's list applies again.

### Server configuration (`#:init-options` and `#:settings`)

Each language server has its own configuration options (code style, extra warnings, where to find things), named and shaped by its own documentation, not by anything LSP-specific. HUME only hands the hash over, and servers differ on how they want it. Check your server's docs for the option names, and try `#:init-options` first.

**`#:init-options` is sent once, when the server starts up.** Most servers, rust-analyzer and gopls among them, pick up their configuration this way:

```scheme
;; gopls reads its own option names directly off the hash you pass here —
;; no extra nesting needed.
(register-lsp-server! "gopls" #:command "gopls"
  #:init-options (hash "hints" (hash "assignVariableTypes" #t
                                     "parameterNames" #t)
                       "usePlaceholders" #t))
```

**`#:settings` is handed to the server on an ongoing basis, not just once at startup.** HUME sends it right after the server is ready, and keeps it to answer when the server later asks for a piece of it by name. Some servers read what's handed to them upfront, some ask for a named piece, some do both. When in doubt, shape the hash the way your server's docs show for a settings file, and pass it as both:

```scheme
;; typescript-language-server reads its "typescript" and "javascript"
;; keys from what's handed to it, without asking for them by name.
(register-lsp-server! "typescript-language-server"
  #:command "typescript-language-server" #:args '("--stdio")
  #:settings (hash "typescript" (hash "inlayHints" (hash "parameterNames" (hash "enabled" "all")))))
```

If a server asks for a named piece that isn't in your `#:settings` hash, HUME answers that nothing is configured for it, which is not an error. Well-behaved servers, including gopls and rust-analyzer, keep whatever configuration they already have.

Changing either and running `:reload-config` updates what HUME has stored, but a running server keeps the configuration it started with until it restarts, and HUME says so once in the message log. Run `:lsp-restart` (or reinstall the server) to push the change.

## Commands and keys

| Key   | Command                    | Effect |
|-------|------------------------------|--------|
| `K`   | `lsp/hover`                  | Show docs for the symbol under the cursor (Vim's own keyword-lookup key) |
| `g d` | `lsp/goto-definition`        | Jump to the symbol's definition |
| `g D` | `lsp/goto-declaration`       | Jump to the symbol's declaration |
| `g y` | `lsp/goto-type-definition`   | Jump to the symbol's type's definition |
| `g i` | `lsp/goto-implementation`    | Jump to the symbol's implementation |
| `z r` | `lsp/references`             | List every reference to the symbol |
| `G R` | `lsp/rename`                 | Rename the symbol under the cursor everywhere it's used |
| `z a` | `lsp/code-actions`           | Show fixes and refactors available at the cursor |
| `z s` | `lsp/document-symbols`       | Pick a symbol of this file (functions, types, and so on) and jump to it |
| `z S` | `lsp/workspace-symbols`      | Search the symbols of the whole project as you type and jump to one |
| `g n` | `lsp/goto-next-diagnostic`       | Jump to the next error/warning after the cursor (wraps) |
| `g p` | `lsp/goto-prev-diagnostic`       | Jump to the previous error/warning before the cursor (wraps) |
| —     | `:diagnostics`               | List every diagnostic in the buffer, each with the tool that reported it |
| —     | `:format-source`             | Format the selected lines if every selection spans one or more whole lines, the whole buffer if none do, or (with a warning) nothing if it's a mix of the two |
| —     | `:toggle-inlay-hints`        | Show or hide inlay hints in the current buffer |
| `Ctrl-Space` (Insert) | `completion-trigger`        | Show completions at the cursor (an editor key, not this plugin's; the plugin supplies the server's candidates) |

Jumping to a definition, declaration, type, implementation, or reference in another file
opens that file as a buffer; `Ctrl-o` jumps back. A goto with more than one match opens a
list to pick from instead of jumping, and `z r` (references) always opens the list, even for
a single hit.

That list (the same one `:diagnostics` opens) is a scrolling drawer across the bottom of
the screen. `Ctrl-d`/`Ctrl-u` page it half a screen at a time, `Down`/`Up` move one row,
`Enter` jumps to the highlighted row, and `Esc` closes it. Every other key, including `j`/`k`,
reaches the buffer underneath. The drawer stays open while you edit, so you can browse a long
references list and edit without losing your place in either.

The list follows your edits. When the number of lines changes in the file you asked from, or
in a file it lists, or an edit touches or moves a listed line, HUME asks the language server
again about the same symbol once you pause typing, wherever your cursor is, and replaces the
rows. If nothing is found any more, the drawer closes with a message.

Typing while a completion menu is open narrows it. `Tab` and `Down` move to the next entry,
`Shift-Tab` and `Up` to the previous, `Enter` accepts the highlighted one, and `Esc`
dismisses the menu. Signature help pops up as you type an argument list for a function the
server knows about, and inlay hints (see below) appear inline once enabled.

After `g n` or `g p` jumps, a popup shows the full diagnostic message until your next
keypress or mouse action. Each line with a problem also gets a short summary at its end.
While you type in Insert mode, the underline and summary are hidden on the cursor's line,
and return when you leave Insert mode or move to another line. Set
`lsp-diagnostics-on-insert-line` to `#t` to keep them showing.

## Settings

The `lsp-*` options configure HUME's built-in language-server support, shared by every
plugin that talks to a language server. They are global options, described in full in
[Global options](settings.md#global-options), and you set them like any other:

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
| `format-max-ranges` | integer ≥ 1 | `16` | Above this many disjoint ranges, `:format-source` warns and formats nothing instead of sending one request per range (a server that batches ranges into a single request isn't capped) |

### Format on save

Off by default. To format the whole buffer every time you save, before the file is written, add this to your `init.scm`:

```scheme
(register-hook! 'on-before-buffer-save
  (lambda (pane done) (call! "lsp/format-document" pane done)))
```

The save waits for the language server's answer, and `:wa` also formats buffers that no pane is showing. If there is no server, or it fails or doesn't answer within `hook-before-save-timeout-ms` (see [Global options](settings.md#global-options)), the buffer is saved as it is. An answer that arrives later still formats the buffer, leaving it modified.

## Managing servers

Commands for a server that's already running. To install, browse the catalog, or remove a
server from disk, see [Installing servers](#installing-servers).

| Command | Effect |
|---------|--------|
| `:lsp-status` | Show every running server and its state, each buffer's servers, and the servers you stopped |
| `:lsp-stop [name]` | Stop a server (default: every server on the focused buffer); with a name, every running copy of that server. It stays stopped, even for new buffers and server-list changes, until `:lsp-restart`, `:reload-config` or a new `register-lsp-server!` of its name |
| `:lsp-restart [name]` | Stop and respawn a server, or start one that was stopped (default: every server on the focused buffer) |

A server stops by itself once no open buffer uses it: after you close its last buffer, or
when a server list, or a buffer's language, no longer includes it. Opening a matching file starts it
again. A server that crashed stays down for its buffers until you run `:lsp-restart`.

Server output and protocol errors show in `:messages`.

A plugin can also send a server requests of its own, such as a server extension `core:lsp` doesn't use. See [Sending a custom request](lsp-api.md#sending-a-custom-request).
