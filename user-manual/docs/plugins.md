# Plugins

HUME plugins are written in Scheme (Steel dialect) and managed by **PLUM** — the HUME **PLU**gin **M**anager, included with the editor.

PLUM is not different from any other plugin, so you must load it — add `(load-plugin "core:plum")` to your `init.scm`.

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

## PLUM commands

| Command | Effect |
|---------|--------|
| `:plum-install` | Install all declared plugins not yet on disk |
| `:plum-cleanup` | Remove on-disk plugins no longer declared |
| `:plum-update` | Pull latest in every installed third-party plugin |
| `:plum-list` | Show declared/installed/orphan/missing plugins |

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
  "Print a greeting from my plugin."
  (lambda ()
    (log! 'info "Hello from my plugin!")))
```

This registers `:hello` as a typed command.

### Defining commands

```scheme
(define-command! "command-name"
  "One-line description shown in command help."
  (lambda ()
    ...))
```

Registers a typed command available as `:command-name`. The second argument is a doc string shown in command help; the function is called when the command is dispatched.

For commands that stream subprocess output to the terminal (installers, git operations), add the `#:inline-output #t` keyword — HUME exits the alt-screen so output is visible, then waits for a keypress before returning. HUME exposes narrow shell builtins (`git-clone`, `git-pull`, `git-clone-rev`, `curl-fetch`) rather than a generic `system` call:

```scheme
(define-command! "fetch-config"
  "Clone the team config repo into the data directory."
  (lambda ()
    (git-clone "https://github.com/team/hume-config.git"
               (path-join (data-dir) "config")))
  #:inline-output #t)
```

For commands that should support dot-repeat (`.`), add `#:repeatable #t`. `#:repeatable` and `#:inline-output` are mutually exclusive:

```scheme
(define-command! "delete-and-repeat"
  "Delete the current selection; dot-repeatable."
  (lambda ()
    (call! "delete-selection"))
  #:repeatable #t)
```

### Calling other commands

Use `(call! ...)` to dispatch other commands from within a plugin:

```scheme
(define-command! "delete-and-save"
  "Delete the selection and write the buffer to disk."
  (lambda ()
    (call! "delete-selection")
    (call! "write")))
```

`call!` routes through the full dispatch system — lazy activation, native Rust commands, and Steel commands are handled uniformly.

### Pending character input

Some commands need a character argument from the user (like surround operations). `(request-wait-char! cmd-name)` arms the pending-char mechanism and dispatches `cmd-name` once the user types a char; `(pending-char)` then reads that char inside the dispatched command:

```scheme
(define-command! "my-surround"
  "Select the surrounding pair, then replace it with the next typed char."
  (lambda ()
    (call! "surround-paren")
    (request-wait-char! "replace")))
```

The status bar shows a pending indicator while waiting.

### Register prefix

To make subsequent `(call! …)` invocations in a command body target a specific register, call `set-register-prefix!` with a single-character register name (`0`–`9`, `k`, `c`, `b`):

```scheme
(define-command! "paste-kill-ring-after"
  "Paste the kill-ring head after the selection (same as \"kp)."
  (lambda ()
    (set-register-prefix! "k")
    (call! "paste-after")))
```

The prefix persists for the rest of the command body. The status bar shows `"` while the register prompt is active.

### Hooks

Plugins react to editor lifecycle events by registering a hook handler with `register-hook!`. It must be called at the top level or inside a plugin body — not from a command body:

```scheme
(register-hook! 'on-buffer-save
  (lambda (buffer-id)
    (log! 'info (string-append "saved buffer " (to-string buffer-id)))))
```

Available hooks and their lambda signatures:

| Hook | Fires when | Lambda args |
|------|------------|-------------|
| `on-buffer-open` | A buffer is opened | `(buffer-id)` |
| `on-buffer-close` | A buffer is about to close | `(buffer-id)` |
| `on-buffer-save` | A buffer is saved | `(buffer-id)` |
| `on-mode-change` | The editor mode changes | `(old new)` — mode strings |
| `on-language-set` | A buffer's language is detected or changed | `(buffer-id lang)` — `lang` is a string or `#f` |

For lazy plugins, declare the events that should trigger activation via `#:events` on `declare-plugin` instead (see [How plugins are loaded](#how-plugins-are-loaded)).

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
