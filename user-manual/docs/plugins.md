# Plugins

HUME plugins are written in Scheme (Steel dialect) and managed by **PLUM** — the HUME plugin manager, included with the editor.

## Installing a plugin

Add a `declare-plugin` or `load-plugin` call to your `init.scm`:

```scheme
(declare-plugin "username/repo-name" #:commands '("my-cmd"))
(load-plugin "username/my-theme")
```

On next launch, PLUM clones the plugin from GitHub, loads it, and makes its commands and key bindings available.

See [How plugins are loaded](#how-plugins-are-loaded) for the difference between the two verbs.

## Plugin status

```
:plugin-status
```

Shows all declared plugins, whether they loaded successfully, and which commands they registered.

## Reloading configuration

```
:reload-config
```

Reloads `init.scm` from scratch. Useful after editing your config without restarting the editor.

## How plugins are loaded

There are two ways to bring a plugin into the editor from `init.scm`:

| Verb | Timing |
|------|--------|
| `(declare-plugin "name" #:commands ...)` | **Lazy** — body deferred until first use |
| `(load-plugin "name")` | **Eager** — body runs during startup |

**Eager plugins** (`load-plugin`) evaluate their body immediately. Use this for plugins that install options, hooks, or key bindings that must be in place from the first keystroke — themes, paste-style overrides, or anything without a natural "first use" trigger.

**Lazy plugins** (`declare-plugin`) record a *manifest* of what the plugin offers, but don't evaluate the body until the first activation entry is exercised. This keeps startup fast: a Rust formatting plugin whose commands you might never call costs nothing until you do.

A lazy plugin must declare at least one activation entry. Without one, the plugin could never activate:

- **`#:commands`** — command names the plugin provides. HUME creates placeholder stubs so the names appear in `:` Tab completion immediately; the first dispatch triggers real definition.
- **`#:events`** — lifecycle hooks that trigger loading (e.g., `'on-buffer-open`).
- **`#:languages`** — buffer language names that trigger loading.

## Writing a plugin

A plugin is a Scheme file placed in PLUM's managed directory. The simplest plugin:

```scheme
(define-command! "hello"
  (lambda ()
    (log-info "Hello from my plugin!")))
```

This registers `:hello` as a typed command.

### Defining commands

```scheme
(define-command! "command-name"
  (lambda ()
    ...))
```

Registers a typed command available as `:command-name`. The function is called when the command is dispatched.

For commands that produce subprocess output (formatters, installers, linters), use `define-command-inline-output!` — this exits the alt-screen so output is visible:

```scheme
(define-command-inline-output! "format"
  (lambda ()
    (system "rustfmt" (buffer-path))))
```

For commands that should support dot-repeat (`.`), use `define-command-repeatable!`:

```scheme
(define-command-repeatable! "indent-two"
  (lambda ()
    (call! "indent")))
```

### Calling other commands

Use `(call! ...)` to dispatch other commands from within a plugin:

```scheme
(define-command! "delete-and-save"
  (lambda ()
    (call! "delete")
    (call! "write")))
```

`call!` routes through the full dispatch system — lazy activation, native Rust commands, and Steel commands are handled uniformly.

### Pending character input

Some commands need a character argument from the user (like surround operations). Use `(request-wait-char!)` to arm the pending-char mechanism:

```scheme
(define-command! "my-surround"
  (lambda ()
    (request-wait-char!)
    (let ((ch (pending-char)))
      (log-info "User typed: " ch))))
```

`(pending-char)` reads the character the user typed after the command was dispatched. The status bar shows a pending indicator while waiting.

### Register prefix

To read a user-chosen register before an operation:

```scheme
(set-register-prefix!)
(let ((reg (pending-register)))
  ...)
```

This arms the register prompt. The status bar shows `"` while waiting for the register name.

### Defining events

Plugins can hook into editor lifecycle events:

```scheme
(define-event! "my-init"
  (lambda ()
    (log-info "Buffer opened: " (buffer-name))))
```

Available event types:

| Event | Fires when |
|-------|-----------|
| `on-buffer-open` | A buffer is opened |
| `on-buffer-close` | A buffer is about to close |
| `on-buffer-save` | A buffer is saved |
| `on-mode-change` | The editor mode changes |
| `on-language-set` | A buffer's language is detected or changed |

Register a plugin to respond to events:

```scheme
(declare-plugin "my-plugin" #:events '(on-buffer-open on-language-set))
```

### Sandboxed filesystem

Plugins have access to sandboxed filesystem operations restricted to the data directory:

| Function | Description |
|----------|-------------|
| `(make-dir path)` | Create a directory |
| `(delete-dir path)` | Delete a directory |
| `(delete-file path)` | Delete a file |
| `(list-dir path)` | List directory contents |
| `(path-exists? path)` | Check if a path exists |

These are sandboxed to `data/plugins/`, `data/grammars/`, and `runtime/plugins/`.

## Bundled core plugins

HUME ships with several built-in plugins that are always available:

| Name | Description |
|------|-------------|
| `core:plum` | Plugin and grammar manager (commands: `:plum-install-grammar`, `:plum-list`, etc.) |
| `core:helix-surround` | Helix-style surround (ms = wrap, md = delete, mr = replace) |
| `core:classic-paste` | Classic paste commands (`:classic-ring-after`, `:classic-clipboard-before`, etc.) |

These plugins are declared in the default `init.scm` and loaded on demand.

## PLUM (plugin and grammar management)

The `:plum-*` commands are provided by the bundled **`core:plum`** plugin. They are only available when that plugin is loaded — add `(load-plugin "core:plum")` to your `init.scm` if they are not present.

| Command | Effect |
|---------|--------|
| `:plum-install-grammar` | Install tree-sitter grammar for current buffer's language |
| `:plum-update-grammar` | Re-clone and recompile grammar for current buffer's language |
| `:plum-ensure-grammars` | Install grammars from a list, skip compiled |
| `:plum-list-grammars` | Show known/installed/orphan/missing grammars |
| `:plum-cleanup-grammars` | Delete orphan compiled grammar files |
| `:plum-install` | Install all declared plugins not yet on disk |
| `:plum-cleanup` | Remove on-disk plugins no longer declared |
| `:plum-update` | Pull latest in every installed third-party plugin |
| `:plum-list` | Show declared/installed/orphan/missing plugins |

## Example plugin

```scheme
;; ~/.config/hume/plugins/my-utils.scm

(define-command! "greet"
  (lambda ()
    (log-info "Hello! The current file is: " (buffer-name))))

(define-command-repeatable! "duplicate-line"
  (lambda ()
    (call! "copy-selection-on-next-line")))

(define-event! "on-save-notify"
  (lambda ()
    (log-info "Saved: " (buffer-name))))
```