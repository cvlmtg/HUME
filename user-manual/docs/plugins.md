# Plugins

HUME plugins are written in Scheme (Steel dialect) and managed by **PLUM**, a bundled core plugin — see [Core Plugins](core-plugins.md#plum) for what it is and how to enable it.

## Installing a plugin

Add a `declare-plugin` or `load-plugin` call to your `init.scm`:

```scheme
(declare-plugin "username/repo-name" #:commands '("my-cmd"))
(load-plugin "username/my-theme")
```

On next launch, PLUM clones the plugin from GitHub, loads it, and makes its commands and key bindings available.

See [How plugins are loaded](#how-plugins-are-loaded) for the difference between the two verbs.

If a plugin supports configuration, pass it with `#:config`:

```scheme
(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'off))
```

See [Configuring a plugin](#configuring-a-plugin) for what a plugin does with this value, and the plugin's own docs for which keys it understands.

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

A lazy plugin needs at least one activation entry, or it could never activate. Declare them yourself:

- **`#:commands`** — command names the plugin provides. HUME creates placeholder stubs so the names appear in `:` Tab completion immediately; the first dispatch triggers real definition.
- **`#:events`** — lifecycle hooks that trigger loading (e.g., `'on-buffer-open`).
- **`#:languages`** — buffer language names that trigger loading.

...or, if the plugin ships its own defaults, leave all three off:

```scheme
(declare-plugin "username/repo-name")
```

A bare `declare-plugin` with no activation entries asks the plugin for its own defaults instead of erroring — see [Default activation](#default-activation) if you're writing a plugin and want to support this.

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

For commands that stream subprocess output to the terminal (installers, git operations), add the `#:inline-output #t` keyword. The alt-screen opens on the command's first real output — not eagerly at the start — so a run that produces no output (an already-up-to-date check, a validation error) never flashes an empty screen or waits on an unneeded keypress. Once something is printed, HUME exits the alt-screen so it's visible, then waits for a keypress before returning.

Plugins run with the same privileges as HUME itself, so any Scheme process/filesystem function is available — there's no separate "shell builtin" layer. The one exception: inside an `#:inline-output` command, spawn subprocesses whose output should reach the terminal via `run-inline-output!` rather than a raw `spawn-process`/`command` call — it isolates the child into its own process group so a Ctrl+C meant to interrupt the subprocess doesn't kill HUME too, and it's the trigger that opens the alt-screen:

```scheme
(define-command! "fetch-config"
  "Clone the team config repo into the data directory."
  (lambda ()
    (run-inline-output! "git" (list "clone" "--"
                                     "https://github.com/team/hume-config.git"
                                     (path-join (data-dir) "config"))))
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

When forwarding a `count` argument to another command, a count of `0` means "as if no count was typed" — this is how `move-down`/`move-up` decide between visual-row and buffer-line movement, and it lets a key-bound command that forwards its own `count` behave the same way a native keybinding would.

### Depending on another plugin

::: warning
`call!` with an unknown command name logs a warning and no-ops instead of erroring — a missing plugin dependency shows up as your plugin quietly doing the wrong thing rather than a clear failure.
:::

If your plugin calls another plugin's commands via `call!`, check that the other plugin is loaded before you rely on it, with `(loaded-plugins)` at the top level of your plugin body — before anything that calls into the dependency:

```scheme
(unless (member "core:stdlib" (loaded-plugins))
  (error "my-plugin: requires core:stdlib — load it before my-plugin"))
```

This fails loudly at load time (startup or `:reload-config`), naming exactly what's missing, instead of leaving the bug to surface later at whatever moment the dependent command actually runs.

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

To make subsequent `(call! …)` invocations in a command body target a specific register, call `set-register-prefix!` with a single-character register name (`0`–`9`, `k`, `c`, `b` — see [Register prefix](editing.md#register-prefix) for what each one holds):

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

### Default activation

If most users would activate your plugin the same way, give them a one-liner: put a `declare-plugin` call for your own plugin in a `manifest.scm` file next to your plugin's main file.

```scheme
; manifest.scm
(declare-plugin "username/repo-name"
  #:commands '("my-cmd" "my-other-cmd"))
```

A user who writes `(declare-plugin "username/repo-name")` with no `#:commands`/`#:events`/`#:languages` gets your manifest's entries instead of an error. Passing any activation entry explicitly skips your manifest entirely — the user's list is authoritative, not merged with yours. A plugin with no `manifest.scm` can't be declared this way; users who want to use it lazily must list its activation entries themselves (or you can add one).

If your plugin reacts to a language but can't predict which ones a given user cares about, `#:languages '("*")` matches any buffer with a detected language:

```scheme
; manifest.scm
(declare-plugin "username/repo-name"
  #:languages '("*")
  #:commands '("my-cmd"))
```

`#:config` behaves the same as elsewhere: if the user passes `#:config` to their zero-argument `declare-plugin`, that value wins over anything your manifest passes — read it back the usual way with `(plugin-config)`.

Keep `manifest.scm` to just the `declare-plugin` call — it runs whenever a user's bare `declare-plugin` resolves it, which is not a signal that your plugin is about to load.

### Configuring a plugin

A plugin can read the `#:config` value its user passed to `load-plugin` or `declare-plugin` with `(plugin-config)`. It returns whatever was passed — typically a hash — or an empty hash if nothing was passed:

```scheme
(define cfg (plugin-config))
(unless (and (hash-contains? cfg "disable-binding") (hash-ref cfg "disable-binding"))
  (bind-key! 'normal "C" "my-command"))
```

Document the keys your plugin understands so users know what to pass.

### Filesystem and processes

Plugins are trusted code: they can read and write any file, and spawn any process, just like any other Scheme program. There's no separate sandboxed subset of the filesystem — use Scheme's own functions directly (`open-input-file`, `create-directory!`, `delete-file!`, `read-dir`, `path-exists?`, and so on) for file access, and `command`/`spawn-process`/`wait` for running external tools.

A few extra functions cover things Scheme has no way to know on its own:

| Function | Description |
|----------|-------------|
| `(data-dir)` | HUME's data directory, or `#f` if unavailable |
| `(runtime-dir)` | HUME's runtime directory, or `#f` if unavailable |
| `(path-join seg…)` | Join path segments with the OS-native separator |

Only install or overwrite files under `(data-dir)` unless you have a specific reason to go elsewhere — that's where HUME expects a plugin's own data (installed grammars, downloaded servers, plugin state) to live.

## Bundled core plugins

HUME ships several built-in plugins — see [Core Plugins](core-plugins.md) for the full list and what each does.
