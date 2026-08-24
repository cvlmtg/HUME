# Plugin Architecture: Loading, Activation, and Isolation

HUME plugins extend the editor by registering commands and hooks. This document explains
how plugins are loaded, when their code runs, and how they interact with each other.

For ownership and conflict rules see [Plugin Attribution: Who Owns What](plugin-attribution.md).

---

## Two verbs, two timings

There are two ways to bring a plugin into the editor from `init.scm`:

```scheme
(load-plugin "alice/my-theme")           ; eager — body runs now, at startup
(declare-plugin "alice/lazy-thing"       ; lazy — body deferred until first use
  #:commands '("my-cmd"))
```

**Eager plugins** (`load-plugin`) evaluate their body immediately. Use this for a plugin
whose only possible trigger is something its own body sets up — a key binding it adds or
overrides, an option, a hook — since nothing outside the plugin could ever fire first and
wake it.

**Lazy plugins** (`declare-plugin`) don't evaluate their body until the first activation
entry is exercised. This keeps startup fast: a Rust formatting plugin whose commands you
might never actually call in a session costs nothing until you do.

---

## Passing configuration

Both verbs accept an optional `#:config` value — typically a hash — that the plugin
body can read back for itself:

```scheme
(load-plugin "alice/my-theme" #:config (hash "variant" "dark"))
```

```scheme
; alice/my-theme/plugin.scm
(define cfg (plugin-config))
(if (and (hash-contains? cfg "variant")
         (equal? (hash-ref cfg "variant") "dark"))
    (load-dark-palette)
    (load-light-palette))
```

`(plugin-config)` returns the calling plugin's own config — never another plugin's —
while its body is being evaluated: instantly for an eager plugin, or at activation
time for a lazy one (the value recorded at `declare-plugin` time is kept until then,
even much later in the session). Called from anywhere else, such as inside a command
the plugin registers, it returns an empty hash — commands run after the body has
already finished, outside that window. Read the config once at the top of the body,
as `cfg` does above, and capture whatever a command needs from it in a `define`. A
plugin author decides what keys their config hash understands and documents them for
users.

When the same plugin is mentioned more than once, the two verbs resolve differently:
duplicate `declare-plugin` calls keep the *first* config, while `load-plugin` always
overwrites whatever was recorded. So a bare `(load-plugin "x")` after a configured
declare deliberately runs the body with the empty default — the most recent explicit
load wins.

---

## The manifest and the body

When HUME processes a `declare-plugin` call with at least one activation entry, it
records a *manifest* — a description of what the plugin offers — and nothing else. The
plugin file is not read, no code runs. A `declare-plugin` with *no* entries instead
evaluates the plugin's own small manifest file, which declares the plugin with its
author-chosen default activation entries — a bare declare lets the plugin supply its own
triggers.

The manifest contains three optional lists:

| Keyword | Meaning |
|---------|---------|
| `#:commands` | Command names the plugin will register |
| `#:events` | Lifecycle hooks that should trigger loading — a list of quoted symbols |
| `#:languages` | Buffer language names that should trigger loading |

When one of those entries is exercised for the first time — a listed command is
dispatched, a listed hook fires, a listed language is set — HUME loads the plugin body.
The body is evaluated exactly once. It typically calls `define-command!`, `register-hook!`,
and `bind-key!` to wire everything up; after that, commands and hooks remain active until
`:reload-config` rebuilds from scratch.

---

## Activation entries

A lazy plugin needs at least one activation entry — with none, there is no moment that
would ever trigger loading. You can supply the entries yourself, or omit them entirely
and let the plugin's bundled manifest supply its author-chosen defaults (an error if the
plugin ships none).

The three entry types serve different loading patterns:

**`#:commands`** is the most common. Declare the command names the plugin will register;
HUME creates placeholder stubs so those names appear in `:` command-line completion
immediately. The first
time someone dispatches one, the plugin body runs and replaces the stub with the real
implementation.

```scheme
(declare-plugin "alice/rust-tools" #:commands '("rust-check" "rust-fmt"))
(bind-key! 'normal "<space>r" "rust-check")
; pressing <space>r the first time loads alice/rust-tools, then runs rust-check
```

**`#:events`** defers loading until a lifecycle hook fires. Useful for plugins that
react to buffer events globally (not just for a specific language). Hook names are
symbols, the same form `register-hook!` takes — not strings, unlike `#:commands` and
`#:languages`:

```scheme
(declare-plugin "alice/autosave" #:events '(on-buffer-open))
; body runs the first time any buffer is opened
```

**`#:languages`** defers loading until the buffer language is set to one of the
named languages. This is the preferred pattern for language-specific plugins (see
[Language Identity and Detection](language-identity.md) for how languages are detected):

```scheme
(declare-plugin "alice/rust-tools" #:languages '("rust"))
; body runs the first time a buffer language is set to "rust"
```

The special name `"*"` is an any-language wildcard: the body runs the first time a
buffer's language is set to *anything* — for plugins that work with every language
rather than a list they could enumerate.

Use `#:languages` rather than `#:events '(on-language-set)` when you only care about
one language. A `on-language-set` event fires for *every* language, so a Rust plugin
declared that way would load the moment you open a PHP file. `#:languages` names only the
languages you care about, keeping the plugin dormant until one of them appears.

The full set of lifecycle hooks, for reference:

| Hook | Fires |
|------|-------|
| `on-buffer-open` | A buffer is opened |
| `on-buffer-close` | A buffer is closed |
| `on-buffer-save` | A buffer is written to disk |
| `on-buffer-enter` | The focused buffer changes |
| `on-focus-gained` | The terminal regains focus |
| `on-mode-change` | The editor mode changes (e.g. entering insert) |
| `on-language-set` | A buffer's language is set or cleared |
| `on-lsp-attach` | A language server attaches to a buffer |
| `on-lsp-detach` | A language server detaches from a buffer |
| `on-diagnostics-changed` | Diagnostics arrived (or cleared) for a buffer |
| `on-viewport-change` | The visible region settled after a scroll or resize |
| `on-trigger-char` | A registered trigger character was typed in insert mode |
| `on-completion-accept` | A completion candidate was accepted |
| `on-completion-refilter` | An incomplete completion list needs a fresh request as typing continues |
| `on-option-change` | A global setting changed (`:set global`, `set-option!`, `:theme`) |
| `on-text-changed` | A buffer's text changed — edits, undo/redo, and `:e!` reload alike, coalesced into one fire per triggering command rather than one per underlying mutation |

Hooks never fire mid-command. Whatever triggers one — a command you ran, a
language server responding, a timer, the terminal regaining focus — queues
it, and the editor drains the queue once it reaches a stable point: after
your command finishes, or, for background triggers, the next time the editor
checks in on its own. If a handler itself does something that would trigger
another hook (switching buffers from inside an `on-buffer-save` handler, say),
that hook queues too and drains in the same pass — you never need an extra
keypress to see the cascade finish. Plugins always observe the editor in a
stable state, not mid-edit.

One caveat: the named language must already be known to the editor when a buffer is
opened. If a plugin is the sole definer of its own activation language — registering it
inside its own body with `define-language!` — it can never load. The body needs a buffer
in that language to trigger activation, but the language can't be set on any buffer until
the body runs. This is a permanent deadlock for the session; HUME will flag it at startup
with a warning visible in `:messages`.

The fix is to separate identity from behavior: define the language eagerly in `init.scm`
so its identity exists from startup, then declare the tooling lazily:

```scheme
; init.scm
(define-language! "mylang" '("ml"))                           ; identity — eager
(declare-plugin "alice/mylang-tools" #:languages '("mylang")) ; behavior — lazy
```

Once the body has run (on the first match), register `on-language-set` *inside the body*
if you need to respond to every subsequent language change — `#:languages` is a one-shot
load trigger, not a recurring filter.

```scheme
; alice/rust-tools/plugin.scm
(define-command! "rust-check" "Run cargo check" (lambda () ...))
(register-hook! 'on-language-set
  (lambda (bid lang)
    (when (equal? lang "rust")
      (call! "rust-check"))))
```

---

## Module isolation

Each plugin body loads as its own isolated module. A plain `define` inside a plugin body
is private to that module — another plugin cannot reach it by name.

The only surface a plugin exposes to the rest of the editor is what it registers through
HUME's APIs: commands (`define-command!`), hooks (`register-hook!`), and key bindings
(`bind-key!`). Private helpers remain private.

This means there is no "library plugin" concept in HUME. A plugin cannot export raw
helper functions for other plugins to import. If shared logic is needed, it can be exposed
as a command that other plugins invoke by name.

A plugin *can* split its own body across multiple files using `require`. The
main file pulls in siblings, and private helpers stay private to the
combined module — `plum` itself is structured this way, with grammar
management, plugin management, and shared helpers in separate files all
required by one entry point. What still cannot cross plugin boundaries is
reaching into another plugin's private helpers by name.

---

## Cross-plugin reuse

The only cross-plugin surface is command dispatch. One plugin invokes another by calling
a registered command by name:

```scheme
; alice/formatter/plugin.scm — registers a command
(define-command! "fmt-buffer" "Format current buffer" (lambda () ...))

; bob/on-save-format/plugin.scm — calls it
(register-hook! 'on-buffer-save
  (lambda (bid)
    (call! "fmt-buffer")))
```

`call!` dispatches by command name. If the command belongs to a lazy plugin that hasn't
activated yet, calling it triggers activation inline before the call proceeds.

A key binding, unlike a command, has no activation entry of its own — a plugin's
`bind-key!` calls only take effect once its body has already run. So it's not enough for
a lazy plugin to register a command; something has to be able to *reach* that command
before the plugin's own bindings exist. A plugin that rebinds a key which already does
something is a sharper case of the same problem: until it activates, that key keeps doing
whatever it did before, which is often worse than doing nothing. Either way, if a
plugin's own bindings are the only path to its commands — nothing else can dispatch them,
no event or language would ever fire first — it must use `load-plugin` (eager).

---

## Declaring dependencies

Plugins cannot load or declare other plugins from their own body. Every plugin needed —
including those that exist only to provide commands that other plugins call — must be
declared at the top level of `init.scm`. The order matters: declare or load a dependency
before the plugin that calls its commands, so the command names exist by the time the
dependent plugin is activated.

```scheme
; init.scm — declare dependencies before dependents
(load-plugin "alice/formatter")
(declare-plugin "bob/on-save-format" #:events '(on-buffer-save))
```

`:plugin-status` (alias `:plugins`) lists every declared plugin with its current state
and any activation entries still pending — useful for checking whether dependencies are
loaded before a dependent plugin activates.

---

## See also

- [Plugin Attribution: Who Owns What](plugin-attribution.md) — how HUME tracks which
  plugin registered which command, how conflict detection works, and the load-once model.
