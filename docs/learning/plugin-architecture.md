# Plugin Architecture: Loading, Activation, and Isolation

HUME plugins extend the editor by registering commands and hooks. This
document explains how plugins are loaded, when their code runs, and how they
interact with each other.

## Lazy and eager plugins

A plugin gets into the editor with one call from `init.scm`:

```scheme
(load-plugin! "alice/my-theme")
(load-plugin! "alice/lazy-thing")
```

What happens next depends on what the plugin ships, not on how `init.scm`
names it.

**A plugin with a manifest is lazy.** The manifest lists the entry points
that should wake the plugin: commands, events, languages. The editor records
them and leaves the plugin's code alone until one is used. This keeps startup
fast: a Rust formatting plugin whose commands you never call in a session
costs nothing.

**A plugin without a manifest is eager.** Its code runs during startup. This
suits a plugin whose only trigger is something its own code sets up (a key
binding it adds or overrides, an option, a hook), since nothing outside the
plugin could fire first and wake it.

A config can override a manifest. Declaring entry points for an installed
plugin ahead of its load call replaces the manifest's list, and the plugin
stays lazy. A script beside `init.scm` can be declared the same way, so it
loads lazily without being a full plugin.

## Passing configuration

The load call accepts an optional config, a hash with symbol keys, which the
plugin's own code reads back. It is the only way to pass configuration:

```scheme
(load-plugin! "alice/my-theme" #:config (hash 'variant "dark"))
```

```scheme
; alice/my-theme/plugin.scm
(define cfg (plugin-config))
(if (and (hash-contains? cfg 'variant)
         (equal? (hash-ref cfg 'variant) "dark"))
    (load-dark-palette)
    (load-light-palette))
```

The config belongs to the plugin it was passed to; no other plugin can read
it. The plugin reads it wherever its code runs: its body (at startup for an
eager plugin, at activation for a lazy one, however much later that is), and
every command, hook and callback it defines. Code that belongs to no plugin,
such as `init.scm` itself, gets an error, since an empty answer would look
like "no config passed". A plugin that activates before its load call has run
sees an empty config, so a custom declaration belongs directly above the load
call.

## The manifest and the body

A manifest describes what the plugin offers, and nothing else. Reading it
runs none of the plugin's code. It is a small file beside the plugin, written
by the plugin's author, who chooses the default entry points. A plugin can
have several entries, each naming a different file with its own entry points,
so a heavy part loads only when the feature that needs it is used.

Each entry has up to four optional lists:

| Keyword | Meaning |
|---------|---------|
| `#:commands` | Command names the plugin will register |
| `#:typed-commands` | Command-line (`:`) command names the plugin will register |
| `#:events` | Lifecycle hooks that should trigger loading, as quoted symbols |
| `#:languages` | Buffer languages that should trigger loading |

When one of those is used for the first time (a listed command is
dispatched, a listed hook fires, a listed language is set), HUME loads the
plugin body. The body runs once. It typically calls `define-command!`,
`register-hook!` and `bind-key!`, and what it sets up stays until
`:reload-config` rebuilds everything from scratch.

## Activation entries

A lazy plugin needs at least one activation entry; with none, nothing would
ever load it. The manifest supplies the author's defaults, and you can
replace them with your own. A directory with neither a manifest nor a main
file is an error.

**`#:commands`** is the most common. HUME registers a placeholder for each
listed name at startup, so a key can be bound to it and other code can call
it before the plugin has loaded. The first dispatch runs the plugin body,
which replaces the placeholder with the real command. A name listed under
`#:typed-commands` gets the same treatment and also shows up in `:`
completion right away.

```scheme
(declare-plugin! "alice/rust-tools" #:commands '("rust-check" "rust-fmt"))
(load-plugin! "alice/rust-tools")
(bind-key! 'normal "space r" "rust-check")
; pressing <space>r the first time loads alice/rust-tools, then runs rust-check
```

**`#:events`** defers loading until a lifecycle hook fires, for plugins that
react to buffer events in every language. Hook names are symbols, the form
`register-hook!` takes, unlike the strings of `#:commands` and
`#:languages`:

```scheme
(declare-plugin! "alice/autosave" #:events '(on-buffer-open))
(load-plugin! "alice/autosave")
; body runs the first time any buffer is opened
```

**`#:languages`** defers loading until a buffer's language is set to one of
the named languages. It is the usual pattern for language-specific plugins
(see [Language Identity and Detection](language-identity.md)):

```scheme
(declare-plugin! "alice/rust-tools" #:languages '("rust"))
(load-plugin! "alice/rust-tools")
; body runs the first time a buffer language is set to "rust"
```

The name `"*"` matches any language: the body runs the first time a buffer's
language is set to anything.

Prefer `#:languages` to `#:events '(on-language-set)` when you care about
particular languages. An `on-language-set` event fires for every language, so
a Rust plugin declared that way would load the moment you open a PHP file.

The user manual lists [every lifecycle
hook](../../user-manual/docs/writing-plugins.md#hooks) with its arguments.
They cover buffers opening, closing, saving and gaining focus, mode and
language changes, language servers attaching and sending notifications,
diagnostics, the viewport, typed trigger characters, completion, settings,
text changes and the undo history.

Hooks never fire mid-command. Whatever triggers one (a command, a language
server, a timer, the terminal regaining focus) queues it, and the editor
drains the queue at a stable point: after the command finishes, or, for
background triggers, the next time the editor checks in on its own. A handler
that triggers another hook (switching buffers inside an `on-buffer-save`
handler, say) queues that one too, and it drains in the same pass, so a
cascade never waits for another keypress. Plugins always see the editor
between edits, never in the middle of one.

### A language a plugin defines for itself

The named language must already exist when a buffer is opened. If a plugin is
the only definer of its own activation language, registering it inside its
body with `define-language!`, detection can never pick that language, so no
buffer ever gets it and the body never runs. Only an explicit
`:set buffer language=` breaks the cycle, and HUME warns about such a
declaration at startup in `:messages`.

The fix is to separate identity from behaviour: define the language eagerly
in `init.scm`, then declare the tooling lazily.

```scheme
; init.scm
(define-language! "mylang" '("ml"))                           ; identity, eager
(declare-plugin! "alice/mylang-tools" #:languages '("mylang")) ; behaviour, lazy
(load-plugin! "alice/mylang-tools")
```

`#:languages` is a one-time load trigger, not a filter. A plugin that needs
to respond to every later language change registers `on-language-set` inside
its body:

```scheme
; alice/rust-tools/plugin.scm
(define-command! "rust-check" "Run cargo check" (lambda (pane) ...))
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "rust")
      (call! "rust-check" pane))))
```

## Module isolation

Each plugin body loads as its own module. A plain `define` inside it is
private: another plugin cannot reach it by name.

The only surface a plugin exposes is what it registers through HUME: commands
(`define-command!`), hooks (`register-hook!`) and key bindings (`bind-key!`).
So there are no library plugins: a plugin cannot export helper functions for
others to import. Shared logic is exposed as a command that other plugins
invoke by name.

A plugin can split its own body across several files with `require`. The
main file pulls in the others, and their helpers stay private to the plugin.
`plum` is built this way: its grammar, plugin and theme management are
separate entries that each load on their own commands, sharing helpers from
one common file.

## Cross-plugin reuse

The only cross-plugin surface is command dispatch. One plugin invokes
another's command by name:

```scheme
; alice/linter/plugin.scm registers a command
(define-command! "lint-buffer" "Lint current buffer" (lambda (pane) ...))

; bob/on-save-lint/plugin.scm calls it
(register-hook! 'on-buffer-save
  (lambda (pane)
    (call! "lint-buffer" pane)))
```

If the command belongs to a lazy plugin that hasn't loaded yet, `call!` loads
it first and then makes the call.

A key binding has no activation entry of its own: a plugin's `bind-key!`
calls take effect only once its body has run. So a lazy plugin's commands
must be reachable some other way before its bindings exist. A plugin that
rebinds a key that already does something is a sharper case: until it loads,
the key keeps its old meaning. When a plugin's own bindings are the only way
to reach its commands, it ships no manifest and loads eagerly.

## Declaring dependencies

A plugin cannot load another plugin from its own body. Every plugin needed,
including one that exists only to provide commands for others, is named at
the top level of `init.scm`. Order matters: load a dependency before the
plugin that calls its commands, so the names exist by the time the dependent
plugin activates.

```scheme
; init.scm: dependencies before dependents
(load-plugin! "alice/linter")
(declare-plugin! "bob/on-save-lint" #:events '(on-buffer-save))
(load-plugin! "bob/on-save-lint")
```

`:plugin-status` (alias `:plugins`) lists every plugin named in `init.scm`
with its state and any activation entries still waiting. A plugin that is not
installed shows as `absent`.

---

*See also: [Plugin Attribution: Who Owns What](plugin-attribution.md) for
who owns each command and how conflicts are handled; [Runaway-Script
Protection](runaway-script-protection.md) for what happens when a plugin body
fails or hangs.*
