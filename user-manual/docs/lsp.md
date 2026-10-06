# Language Servers

Everything an IDE gives you, in the terminal. `core:lsp` connects HUME to language servers
for hover docs, go-to-definition, references, diagnostics, rename, formatting, code actions,
signature help, completions, and inlay hints. The companion plugin `core:lsp-install`
installs the servers themselves, so getting a language working is usually one command.

## Setup

Bring in `core:lsp` and `core:lsp-install` from your [`init.scm`](configuration.md) (see
[Core Plugins](core-plugins.md#core-lsp) for how they fit alongside HUME's other bundled
plugins) and make sure a server is registered, and named in the language's list, for the
languages you use. The easiest way to get a server, which does both, is [`:lsp-install`](#installing-servers): run it once per language and it downloads,
verifies, and registers the server in one step, no separate download tool needed. If
you'd rather manage a server yourself (a local build, a version the seeded catalog
doesn't carry, or a `$PATH` copy you want to take precedence), register it by hand
instead. See [Registering a language server](#registering-a-language-server).

```scheme
(load-plugin! "core:stdlib")       ; the plugins depend on it
(load-plugin! "core:lsp")
(load-plugin! "core:lsp-install")  ; :lsp-install and friends; leave it out to manage servers yourself
```

Both plugins load lazily, which keeps startup fast: `core:lsp` activates the first time any file with a recognized language opens, or you run one of its commands directly. `core:lsp-install` registers servers you already installed when a file with a recognized language opens, and loads the rest of itself the first time you run `:lsp-install`, `:lsp-uninstall` or `:lsp-catalog`.

Want activation to only trigger for specific languages, or a smaller set of commands?
Pass `#:languages`/`#:commands`/`#:typed-commands`/`#:events` to `declare-plugin!` before the
`load-plugin!` line, and it uses what you list instead of the defaults:

```scheme
(declare-plugin! "core:lsp"
  #:languages '("rust")   ; only activate for languages you name here
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic"
               "lsp-rename" "lsp-fmt" "lsp-code-actions")
  #:typed-commands '("diagnostics" "format-source"
                      "lsp-status" "lsp-stop" "lsp-restart"))
(load-plugin! "core:lsp")
```

::: warning
`#:events '(on-lsp-attach)` by itself never activates on its own. Nothing
is registered yet, so nothing attaches, so the event that would trigger activation never
fires. List the languages you want servers for in `#:languages`, or list the `lsp-*`
commands in `#:commands`/`#:typed-commands` (as above). Either one gets you a working `core:lsp`.

Completions are a separate case: `Ctrl-Space` and a server's trigger characters both run
through the editor's own `completion-trigger` key, never through one of `core:lsp`'s own
commands, so `#:commands`/`#:typed-commands` alone does not get completions working:
`core:lsp` has to already be active. List the languages you use in `#:languages` to get
completions along with everything else.
:::

Opening a file attaches every server its language's list names, starting each one once per
project root. A language's [list](#choosing-which-server-a-language-uses) is Helix's own once
`core:lsp-install` is loaded, and empty until you set one otherwise.

## Installing servers

`core:lsp-install` downloads and manages language servers for you, the same way [PLUM](core-plugins.md#core-plum)
handles tree-sitter grammars, with no need to track down a binary or install it by hand.

### Prerequisites

Installing a server shells out to a few external tools. Most are already on your system; if
one is missing, the install tells you which one before downloading anything. Depending on the
server:

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

Or name the language directly. Tab completes every language HUME has a seeded server for, and every installable server:

```
:lsp-install rust
```

HUME downloads the pinned release, verifies its checksum, unpacks it, and registers it;
already-open buffers of that language attach immediately, no restart needed. Running
`:lsp-install` again for a server that's already at the latest seeded version never
re-downloads. (A server you've registered by hand under the same name is left alone,
whether or not it is also managed by `:lsp-install`.)

`:lsp-install` installs the first server HUME knows for a language. Some languages have
more than one (Python: ty, ruff, jedi, pylsp, zuban; TOML: taplo, tombi). To install
another, name it. Tab completes every server that can be installed:

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

Lists every server HUME knows about: its languages, its seeded version, and whether it's
installed, out of date, or not installable on your platform (and why). A language a server
only backs second to another is marked `(secondary)`. Not every entry can
be installed for you: many are listed so you can point HUME at a copy you install yourself,
with `register-lsp-server!` below.

### Manage installed servers

```
:lsp-uninstall <name>
```

Shuts down any running client for that server, unregisters it, and removes it from disk. Use
the server's name from `:lsp-catalog`, not the language name, e.g.
`:lsp-uninstall rust-analyzer`, not `:lsp-uninstall rust`. Tab completes every server with an
install directory on disk, including an orphan one no longer in the seeded catalog.

Reinstalling a server that's already running (e.g. to pick up an update) shuts the old client
down first. If the install fails anyway (a locked file on Windows is the usual reason), the
message tells you to run `:lsp-install` again, which normally succeeds the second time.

### Troubleshooting

**`:lsp-install` fails naming a missing tool.** Install it; see the
[prerequisites](#prerequisites) above.

**`:lsp-install` says "not supported on this platform".** The server's own packaging
doesn't support your operating system.

**A server installs but fails to start.** A few servers need more than the install provides:
the Java server (jdtls) needs a JDK 21 or newer and `python3` on your `$PATH` when it runs.

**`:lsp-install` says "not installable".** Not every server HUME knows about can be
auto-installed: some don't publish prebuilt binaries HUME can unpack, are pinned by their
package manager to a git revision rather than a released version, or are only available
through a package manager not yet supported (`opam`, `luarocks`, …). Install it yourself
and register it manually as described in [Registering a language server](#registering-a-language-server)
below.

**A server is on disk but nothing attaches.** `core:lsp-install` is probably not loaded from your `init.scm`. Add `(load-plugin! "core:lsp-install")` to it.

**A server on your `$PATH` isn't the one HUME runs.** `:lsp-install` always spawns the managed
copy, even when the same command name also resolves on `$PATH`. You'll see a note about this
after installing. Register the server manually instead if you want your `$PATH` copy to take
precedence: register it under the same name `:lsp-catalog` shows for the managed server
(`"rust-analyzer"`, not `"rust"`), as described in
[Registering a language server](#registering-a-language-server).

## Registering a language server

Registering by hand is only needed if you're not using
[`:lsp-install`](#installing-servers): a locally built server, a version the seeded
catalog doesn't carry, or a `$PATH` copy you want to take precedence over a managed install.
`register-lsp-server!` itself has no dependency on `core:lsp`, but the
[managing-servers commands](#managing-servers) below (`:lsp-status`, `:lsp-stop`,
`:lsp-restart`) are `core:lsp` commands, so a manually registered server still needs
`core:lsp` loaded to inspect, stop, or restart it. A manual `register-lsp-server!`
call under the same name as a seeded, installed server replaces it, whether it comes
before or after `(load-plugin! "core:lsp")` in your init.scm; order doesn't matter.

A registration says how to start a server. A server serves a language only once that
language's server list names it, so the examples that follow list each server for its
languages with a [`set-language-servers!`](#choosing-which-server-a-language-uses) call. The project root
comes from the language's own root markers, which the bundled languages already carry
(`Cargo.toml` for Rust, `go.mod` for Go, and so on). `register-lsp-server!` takes:

| Argument | Meaning |
|----------|---------|
| name | The server's name. Registering the same name again replaces its registration, and `:lsp-stop`/`:lsp-restart` take it. No spaces |
| `#:command` | The executable to run |
| `#:args` | Extra command-line arguments, if the server needs them |
| `#:init-options` | Server configuration, sent once at startup; see below |
| `#:settings` | Server configuration, handed over on an ongoing basis; see below |
| `#:env` | Extra environment variables for the server process, as a list of `("NAME" . "value")` pairs, added on top of the environment HUME itself runs in, never replacing it |

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
order. With `core:lsp-install` loaded, every language it knows starts with Helix's list for
it. Any other language has no list until you set one, so a server you register yourself
serves nothing until a list names it. Diagnostics from every server
show together, completion lists every server's items, code actions list every server's
actions (with the server's name beside each when several offer some), goto and references
combine every server's locations, and inlay hints come from all of them. Hover, signature
help, rename and formatting use the first server in the list that is running and supports
the feature. Give the names in the order you want:

```scheme
(set-language-servers! "python" '("pyright" "ruff"))
```

A list names the only servers a language uses, so adding a server means restating the whole
list. This runs ESLint beside typescript-language-server for JavaScript and TypeScript:

```scheme
(for-each
  (lambda (language)
    (set-language-servers! language
      '("typescript-language-server" "vscode-eslint-language-server")))
  '("javascript" "jsx" "typescript" "tsx"))
```

`:lsp-status` shows which languages each running server serves. A name that isn't registered
yet takes its place in the list once it registers. An entry can
also limit a server to some features, with `only-features` or `except-features` (give one,
not both), using the feature names Helix uses in its own `languages.toml`:

```scheme
(set-language-servers! "python"
  (list "pyright" (hash 'name "ruff" 'only-features '(format diagnostics))))
```

A server limited this way contributes only those features: with `'(format diagnostics)`,
ruff's diagnostics are shown and its other answers are not used.

Plugins can ship a list of their own with `set-default-language-servers!`, which takes the
same arguments. It applies only while you haven't set a list for that language. Passing `#f`
instead of a list clears what you set.

### Server configuration (`#:init-options` and `#:settings`)

Language servers each have their own configuration options (code style, extra warnings, where to find things), and most of them read the *shape and names* of those options straight from their own documentation, not from anything LSP-specific. HUME just needs to hand the hash over in the right way, and servers differ on which way that is. Check your server's own docs for the option names, and try `#:init-options` first.

**`#:init-options` is sent once, when the server starts up.** This is how most servers actually pick up their configuration, including well-known ones like rust-analyzer and gopls:

```scheme
;; gopls reads its own option names directly off the hash you pass here —
;; no extra nesting needed.
(register-lsp-server! "gopls" #:command "gopls"
  #:init-options (hash "hints" (hash "assignVariableTypes" #t
                                     "parameterNames" #t)
                       "usePlaceholders" #t))
```

**`#:settings` is handed to the server on an ongoing basis instead of just once at startup:** HUME sends it right after the server is ready, and keeps it on hand to answer if the server asks for a specific piece of it later by name. Which of those two a given server actually pays attention to depends on the server (some read what's handed to them upfront, some ask for a named piece, some do both), so when in doubt, shape the hash to match whatever your server's own docs show for a settings file, and pass it as both:

```scheme
;; typescript-language-server reads its "typescript" and "javascript"
;; keys from what's handed to it, without asking for them by name.
(register-lsp-server! "typescript-language-server"
  #:command "typescript-language-server" #:args '("--stdio")
  #:settings (hash "typescript" (hash "inlayHints" (hash "parameterNames" (hash "enabled" "all")))))
```

If a server does ask for a specific named piece and it isn't in your `#:settings` hash, that's a plain "nothing configured for this" answer, not an error. Well-behaved servers, including gopls and rust-analyzer, take that in stride and keep whatever configuration they already have.

Changing either and running `:reload-config` updates what HUME has stored, but an already-running server keeps the configuration it started with until it restarts, and HUME says so once in the message log. Run `:lsp-restart` (or reinstall the server) to push the change.

## Commands and keys

| Key   | Command                    | Effect |
|-------|------------------------------|--------|
| `K`   | `lsp-hover`                  | Show docs for the symbol under the cursor (Vim's own keyword-lookup key) |
| `g d` | `lsp-goto-definition`        | Jump to the symbol's definition |
| `g D` | `lsp-goto-declaration`       | Jump to the symbol's declaration |
| `g y` | `lsp-goto-type-definition`   | Jump to the symbol's type's definition |
| `g i` | `lsp-goto-implementation`    | Jump to the symbol's implementation |
| `z r` | `lsp-references`             | List every reference to the symbol |
| `G R` | `lsp-rename`                 | Rename the symbol under the cursor everywhere it's used |
| `z a` | `lsp-code-actions`           | Show fixes and refactors available at the cursor |
| `g n` | `goto-next-diagnostic`       | Jump to the next error/warning after the cursor (wraps) |
| `g p` | `goto-prev-diagnostic`       | Jump to the previous error/warning before the cursor (wraps) |
| —     | `:diagnostics`               | List every diagnostic in the buffer |
| —     | `:format-source`             | Format the selected lines if every selection spans one or more whole lines, the whole buffer if none do, or (with a warning) nothing if it's a mix of the two |
| `Ctrl-Space` (Insert) | `completion-trigger`        | Show completions at the cursor (an editor key, not this plugin's; the plugin supplies the server's candidates) |

Jumping to a definition, declaration, type, implementation, or reference in another file
opens that file as a buffer; `Ctrl-o` jumps back. A goto with more than one match opens a
list to pick from instead of jumping directly, and `z r` (references) always opens the list,
even for a single hit.

That list (the same one `:diagnostics` opens) is a scrolling drawer across the bottom of
the screen. `Ctrl-d`/`Ctrl-u` page it half a screen at a time, `Shift-Down`/`Shift-Up` move
one row, `Enter` jumps to the highlighted row, and `Esc` closes it. Every other key, including
plain `j`/`k` and the arrow keys, reaches the buffer underneath instead: the drawer stays
open while you keep editing, so you can browse a long references list and edit at the same
time without losing your place in either.

The list follows your edits: when the number of lines changes in the file you asked from, or in a file it lists, HUME asks
the language server again about the same symbol once you pause typing, wherever your cursor
is, and replaces the rows. If nothing is found any more the drawer closes with a message.
A change that keeps the line count doesn't refresh it, so a row on the very line you edited
can sit a few columns off until the next line is added or removed.

Typing while a completion menu is open narrows it. `Tab` and `Down` move to the next entry,
`Shift-Tab` and `Up` to the previous, `Enter` accepts the highlighted one, and `Esc`
dismisses the menu. Signature help pops up automatically as you type an argument list for a
function the server knows about, and inlay hints (see below) appear inline once enabled.

`g n` and `g p` show the full diagnostic message in a popup after they jump; it clears on
your next keypress or mouse action. Each line with a problem also gets a short summary at
its end. While you type in Insert mode, the underline and summary are hidden on the line
the cursor is on and return when you leave Insert mode or move to another line. Set
`lsp.diagnostics-on-insert-line` to `#t` to keep them showing.

## Settings

All `lsp.*` settings are global options, described in full in
[Global options](configuration.md#global-options). Set them the same way as any other
global option:

```scheme
(set-option! "lsp.inlay-hints" #t)
```

### Format on save

Not on by default. Add this to your `init.scm` to run `lsp-fmt` every time you save:

```scheme
(register-hook! 'on-buffer-save
  (lambda (pane) (call! "lsp-fmt" pane)))
```

## Managing servers

Commands for a server that's already running. To install, browse the catalog, or remove a
server from disk, see [Installing servers](#installing-servers).

| Command | Effect |
|---------|--------|
| `:lsp-status` | Show every running server and its state, each buffer's servers, and the servers you stopped |
| `:lsp-stop [name]` | Stop a server (default: every server on the focused buffer); with a name, every running copy of that server. It stays stopped, for new buffers and changes to a server list too, until `:lsp-restart`, `:reload-config` or a new `register-lsp-server!` of its name |
| `:lsp-restart [name]` | Stop and respawn a server, or start one that was stopped (default: every server on the focused buffer) |

A server stops by itself once no open buffer uses it: after you close its last buffer, or after
a server list or a buffer's language no longer includes it. Opening a matching file starts it
again. A server that crashed stays down for its buffers until you run `:lsp-restart`.

Server output and protocol errors are visible in `:messages`.

## Advanced: custom requests

`lsp-request!` isn't limited to the built-in commands above: any plugin can call it to reach
a server extension the built-in feature set doesn't cover. This is how you'd add a command
for rust-analyzer's `rust-analyzer/expandMacro`, which expands the macro under the cursor and
returns its generated code:

```scheme
(define-command! "rust-expand-macro" "Show the expansion of the macro under the cursor."
  (lambda (pane)
    (lsp-request! pane "rust-analyzer/expandMacro" (lsp-position-params pane)
      (lambda (err res)
        (cond
          (err (log! 'error (string-append "expand macro: " (hash-ref err 'message))))
          ((void? res) (log! 'info "Not inside a macro"))
          (else (show-popup! pane (json-ref res "expansion"))))))))
```

The shape is always the same three steps: send a request built from `lsp-position-params` or
`lsp-primary-range-params`, transform the server's response, and hand the result to a UI or store
builtin (`show-popup!`, `show-menu!`, `show-drawer-list!`, `apply-text-edits!`,
`apply-workspace-edit!`, …). `err` and `res` are never both set. Check `err` first and stop
on it, the way every built-in feature does. `res` is a JSON handle: read a field with
`json-ref`/`json-contains?`/`json-list`, not `hash-ref`; a `null` response arrives as void, not `#f`.
`err`, when set, is a hashmap with a `'kind` and a `'message`; read it with `hash-ref`, not
`json-ref`. `'kind` says what went wrong: `'server` when the server answered with an error (its
`'code` is then set too), `'timeout` when no answer came in time, `'unavailable` when no server
could take the request (none supports it, or they are all still starting), `'stopped` when the
server stopped or crashed before it answered, and `'unsent` when the params could not be sent to
the server. The built-in features report `'unavailable` and `'stopped` as a plain message and
every other kind as an error.

When a buffer has several servers, the request goes to the first one in the language's server
order that is running and can answer it. A request for a standard method such as
`textDocument/hover` or `textDocument/formatting` is tied to that method's feature, using the same
feature names as `set-language-servers!` (`'hover`, `'goto-definition`, `'format`, …): a server
whose entry leaves that feature out, or that does not advertise it, is passed over. For any other
method, name the feature with `#:feature`; giving it for a standard method is an error. `(lsp-servers pane #:method "textDocument/hover")` lists
the servers such a request would reach. To reach one server in particular, such as the one an earlier
answer came from, get its value from `(lsp-servers pane)` and pass `#:to server`.
`(lsp-server-name server)` gives its name, `(lsp-capabilities server)` its capabilities, and
`(lsp-capability server #:feature 'completion)` what it advertises for one feature.

`lsp-request-all!` asks every server that can answer and calls back once, when all of them have,
with `(err results)`: one `(hash 'server s 'err e 'result r)` per server, in order. It takes the same
keywords except `#:to`; its params can also be a list of `(server . params)` pairs, giving each
named server its own; the method and `#:feature` still decide whether each of those servers is
sent the request.

The position in `lsp-position-params` (and the ranges in its siblings) is not a line and column
pair you can read: servers count columns in different units, so each server receives the position
in its own units when the request is sent. Add your own keys to the params hash, but take the
position from these builtins rather than building one.

`lsp-request!` also takes four keyword args for requests that fire more than once, or whose answer might arrive after the moment it was asked for has passed. `#:supersede "<key>"` cancels the caller's own previous still-pending request filed under the same key (every server it went to gets `$/cancelRequest` and the old callback never fires), which is how completion's re-request of an incomplete list avoids piling up stale requests as you type. `#:allow-stale #t` lets the callback run even if the buffer has changed since the request was sent, for requests where a slightly-out-of-date answer is still useful. `#:require-focus #t` drops the callback entirely unless the exact pane you called it from is still the one you're looking at, still showing the same buffer, by the time the answer arrives. Hover, signature help, and code actions use it, so a slow answer never pops up over whatever you've moved on to, even if that's just a different split on the same file. `#:tracked token` takes a `track-position!` token for the position the request was asked about: the token follows edits made while the request is pending, and is forgotten once the callback has run, unless the callback calls `keep-tracked-position!`.

A server's response sometimes carries its own position or range rather than the one you sent, such as a related location returned inside `res`, say. Convert it back into a plain buffer offset with `lsp-position->offset`/`lsp-range->offsets` before using it with any editing command; both return `#f` if the buffer has no server attached to convert against.
