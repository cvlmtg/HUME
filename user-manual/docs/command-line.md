# Command Line

Press `:` in Normal mode to open the command line. Type a command name and press `Enter` to run it. `Esc` dismisses without running.

Tab completion is available for command names and, where applicable, their arguments.

## File commands

| Command | Effect |
|---------|--------|
| `:e <path>` | Open file (or reload current file if no path given) |
| `:w` | Save |
| `:w <path>` | Save as |
| `:w!` | Force save (retry with chmod on permission errors) |
| `:wa` | Write all modified buffers to disk |
| `:q` | Quit |
| `:q!` | Force quit |
| `:wq` | Save and quit |
| `:qa` | Quit all buffers |

## Buffer commands

| Command | Aliases | Effect |
|---------|---------|--------|
| `:ls` | — | List buffers |
| `:b <name>` | — | Switch to buffer |
| `:bnext` | `:bn` | Next buffer |
| `:bprev` | `:bp` | Previous buffer |
| `:bd` | — | Close buffer |

## Panes

| Command | Aliases | Effect |
|---------|---------|--------|
| `:split` | `:sp` | Split the current pane horizontally *(not yet implemented)* |
| `:vsplit` | `:vsp` | Split the current pane vertically *(not yet implemented)* |

Pane focus uses the `Ctrl+p` prefix (`Ctrl+p h`/`j`/`k`/`l`/`w`) — see the [Key Reference](key-reference.md).

## Settings

| Command | Effect |
|---------|--------|
| `:set <option>` | Show current value of an option |
| `:set global <option>=<value>` | Set a global option |
| `:set buffer <option>=<value>` | Set an option for the current buffer only |

See [Configuration](configuration.md) for all available options.

## Display

| Command | Aliases | Effect |
|---------|---------|--------|
| `:theme <name>` | — | Load a theme by name; no arg shows current |
| `:theme-debug` | — | Show resolved styles for key UI scopes |
| `:wrap` | `:toggle-soft-wrap` | Toggle soft line wrapping |
| `:messages` | `:mes` | Show message log in a read-only buffer |

Search highlights clear automatically when you press `Esc`. You can also clear them on demand with `:clear-search`.

## Navigation

| Command | Effect |
|---------|--------|
| `:cd <path>` | Change the working directory |
| `:pwd` | Print the current working directory |

## Plugins

| Command | Aliases | Effect |
|---------|---------|--------|
| `:plugin-status` | `:plugins` | Show declared plugins and their load state |
| `:reload-config` | — | Reload `init.scm` from scratch |

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

## Other

| Command | Aliases | Effect |
|---------|---------|--------|
| `:tutor` | — | Open the interactive tutorial |
| `:version` | `:ver` | Show editor version |