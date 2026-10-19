# Plugins

Plugins are written in Scheme, the same language as your config, so the line between configuring HUME and extending it is thin.

This page covers installing and loading plugins other people wrote; [Writing Plugins](writing-plugins.md) covers making your own.

Plugins are installed and updated by **PLUM**, a bundled plugin. See [Core Plugins](core-plugins.md#core-plum) to enable it.

## Installing a plugin

Add a `load-plugin!` call to your [`init.scm`](configuration.md).

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "cvlmtg/grep.hume")
```

::: info
[cvlmtg/grep.hume](https://github.com/cvlmtg/grep.hume) adds one `g /` keybind to open a picker with a live `rg` (or `grep`) search behind it, re-running the search as you type.
:::

`core:stdlib` is grep.hume's own dependency, not something every plugin needs; check each
plugin's docs for what it requires. Then run `:plum-install-plugins` to clone it from GitHub. PLUM never installs anything on its own, so nothing is fetched behind your back at startup; once the plugin is on disk, its commands and key bindings are available from the next launch.

See [How plugins are loaded](#how-plugins-are-loaded) for when a plugin's code runs.

If a plugin supports configuration, pass it with `#:config`:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:vim-keybind" #:config (hash "change-to-eol" 'off))
```

See [Configuring a plugin](writing-plugins.md#configuring-a-plugin) for what a plugin does with this value, and the plugin's own docs for which keys it understands.

## Plugin status

```
:plugin-status
```

Shows one row per plugin entry: whether it is declared (waiting), loaded or failed, and for a declared entry the commands, events and languages still waiting to trigger it. A plugin that loads in pieces has a row for each file, and a local file shows under its `./` path. A plugin named in `init.scm` that is not installed shows as `absent`, so you can tell what `:plum-install-plugins` still has to fetch.

## Reloading configuration

```
:reload-config
```

Reloads `init.scm` from scratch. Useful after editing your config without restarting the editor.

## Recovering from a failed plugin

A plugin that fails to load doesn't stop the rest of `init.scm`: every plugin after it still loads, and every plugin named before it is unaffected. A lazy plugin loads when its first trigger fires, so its failure shows up then instead of at startup. Check `:messages` for the error; it names the plugin and points at the file and line the problem is in. `:plugin-status` shows it as failed alongside everything else, including a plugin whose `manifest.scm` failed before it declared anything.

If the failing plugin came from a git repository (rather than one you're editing yourself), it may already have a fix upstream: run `:plum-update-plugins` to pull the latest version of every installed plugin, then `:reload-config` to try again. This works even for a plugin that failed on the very first line of your config, since loading it never depends on anything named after it.

## How plugins are loaded

`(load-plugin! "name")` is the call to put in `init.scm`. When the plugin loads depends on what the plugin ships:

| Plugin ships | Timing |
|------|--------|
| a `manifest.scm` | **Lazy**: code deferred until first use |
| only a `plugin.scm` | **Eager**: code runs during startup |

**Lazy plugins** ship a *manifest* that lists what the plugin offers (commands, typed commands, events, languages). `load-plugin!` registers those entries and does not run the plugin's code until the first one is exercised. This keeps startup fast: a language-server or formatting plugin whose commands you might never call costs nothing until you do. `load-plugin!` never forces such a plugin to load at startup.

**Eager plugins** have no manifest, so `plugin.scm` runs during startup. This suits a plugin whose only way of being triggered is something its own code sets up (a key binding it adds or overrides, an option, a hook), since nothing else could wake it.

A directory with neither `manifest.scm` nor `plugin.scm` is an error. A missing `core:` plugin logs an error; a missing `user/repo` plugin logs an info message, and `:plum-install-plugins` installs it.

### Choosing your own triggers

`declare-plugin!` lists the entries that wake a lazy plugin. Declare an installed plugin before its `load-plugin!` line to use your triggers instead of its manifest's:

- **`#:commands`**: editor-command names the plugin provides (defined with `define-command!`). HUME creates placeholder stubs so the names are key-bindable and reachable from `call!` immediately; the first dispatch triggers real definition. A key you bind to one of these names in your own `init.scm` works this way: pressing it activates the plugin, then runs the command, so a lazy plugin's commands are key-bindable from the start even though the plugin's *own* bindings aren't in place yet:

  ```scheme
  (declare-plugin! "cvlmtg/grep.hume" #:commands '("grep/picker"))
  (load-plugin! "cvlmtg/grep.hume")
  (bind-key! 'normal "space g" "grep/picker")
  ; pressing <space>g the first time loads cvlmtg/grep.hume, then runs grep/picker
  ```
- **`#:typed-commands`**: typed-command names the plugin provides (defined with `define-typed-command!`). Same stub-then-activate behavior as `#:commands`, but the stub appears in `:` Tab completion and is reachable only from `:`, never from a key or `call!`:

  ```scheme
  (declare-plugin! "cvlmtg/grep.hume" #:typed-commands '("grep"))
  (load-plugin! "cvlmtg/grep.hume")
  ; typing :grep the first time loads cvlmtg/grep.hume, then runs grep
  ```
- **`#:events`**: lifecycle hooks that trigger loading, as a list of symbols (e.g., `'(on-buffer-open)`).
- **`#:languages`**: buffer language names that trigger loading. Triggers on the name being *set* on a buffer (by detection, `:set buffer language=`, or another plugin's `define-language!`), not on the name being a known language yet — so a plugin can't use `#:languages '("foo")` to lazily define `"foo"` itself: HUME sets the buffer's language and reports it as unregistered before your plugin's activation runs, then activates you anyway. The message is informational; call `define-language!` in your activation body and everything downstream (highlighting, LSP) still works from there.

A plugin that already has a declared entry keeps those triggers when `load-plugin!` runs: the manifest is not read and the plugin stays lazy. Put `load-plugin!` right after the `declare-plugin!`, because a plugin that activates during startup before its `load-plugin!` line sees an empty config. A `declare-plugin!` placed after the plugin's `load-plugin!` is ignored, and `:messages` reports it.

For example, to start `core:lsp` only for Rust buffers, or when you use one of its commands:

```scheme
(declare-plugin! "core:lsp"
  #:languages '("rust")   ; only activate for languages you name here
  #:commands '("lsp/hover" "lsp/goto-definition" "lsp/goto-declaration"
               "lsp/goto-type-definition" "lsp/goto-implementation" "lsp/references"
               "lsp/goto-next-diagnostic" "lsp/goto-prev-diagnostic"
               "lsp/rename" "lsp/format-source" "lsp/format-document" "lsp/code-actions"
               "lsp/document-symbols" "lsp/workspace-symbols")
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

### Lazy local files

A script beside your `init.scm` can load lazily too. Name it with a path that starts with `./`, relative to the directory of `init.scm`:

```scheme
(declare-plugin! "./my-lazy.scm" #:commands '("my-cmd"))
```

The name must end in `.scm`, must not contain `..`, and the file must exist, or `init.scm` fails with an error. `#:entry` is not accepted. Inside the file, `(plugin-config)` is an empty hash and `(plugin-dir)` is the file's directory. A local file appears in `(loaded-plugins)` once it has loaded, but not in `(declared-plugins)`.

## Bundled core plugins

HUME ships several built-in plugins; see [Core Plugins](core-plugins.md) for the full list and what each does.
