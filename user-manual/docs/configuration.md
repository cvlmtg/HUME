# Configuration

Options, key bindings, the statusline, plugins, and language servers are all configured in one language, in one file. There's no separate config format to learn — `init.scm` is a Scheme program, so anything you can compute you can configure.

HUME can be configured two ways: the `:set` command for runtime changes during a session, or an `init.scm` file for persistent configuration loaded at startup.

HUME reads persistent configuration from:

- **macOS / Linux:** `$XDG_CONFIG_HOME/hume/init.scm` (defaults to `~/.config/hume/init.scm`)
- **Windows:** `%APPDATA%\hume\init.scm`

If the file does not exist, HUME starts with defaults. If it fails partway through, the error is reported in `:messages` and everything up to that point stays applied — so a broken line late in the file leaves you half-configured rather than back at defaults. Fix it and run `:reload-config` to re-run the file from scratch without restarting.

A reference config ships at `runtime/init.scm.example` (see [File locations](#file-locations)); copy it to the path above if you want a starting point, or see [Example init.scm](#example-init-scm) below.

## Setting options

There are two ways to set an option: the `:set` command for runtime changes, or `set-option!` in `init.scm` for persistent defaults.

### `:set` command

The `:set` command takes a scope and a `key=value` pair. The scope is required:

```
:set global <option>=<value>     set the global default
:set buffer <option>=<value>     override for the current buffer only (takes precedence over global)
:set pane <option>=<value>       override for the current pane only (view-scoped settings)
```

For a buffer option (the [Buffer options](#buffer-options) table below), `:set global` takes effect immediately in every buffer that has no override of its own — not just newly opened ones. `wrap-mode`, the one option `:set pane` accepts, works differently: a `:set global wrap-mode=…` only seeds *new* panes; panes already open keep whatever wrap mode they already had (see [Text wrap](#text-wrap)).

Changes apply to the current session and are not persisted — for persistent configuration, use `init.scm` (below).

### From `init.scm`

```scheme
(set-option! "option-name" value)
```

Sets the global default. The value is a string, boolean, or integer. Callable from `init.scm`, a plugin body, or a command/hook body — anywhere Steel code runs.

```scheme
(set-option! "line-number-style" "absolute")
(set-option! "tab-width" 2)
```

### Reading options from Steel

```scheme
(get-option "option-name")
(get-option bid "option-name")
```

Returns the effective value of an option: called with just an option name, the focused buffer's override if one is set, else the global default. Pass a buffer id first (e.g. inside an `on-language-set` hook, whose handler receives the buffer id as an argument) to read that buffer's value instead of the focused one. Errors on an unknown option name; `language` has no getter — read it with `(buffer-language bid)` instead.

```scheme
(get-option "tab-width")       ; the focused buffer's effective tab-width
(get-option bid "tab-width")   ; bid's effective tab-width
```

## Global options

Set with `:set global <option>=<value>` or `(set-option! "option" value)`. All of these are global-only except `wrap-mode`, which also accepts a per-pane override — see its row below and [Text wrap](#text-wrap).

For a `bool` option, `:set` accepts `true`/`false`, `on`/`off`, `yes`/`no`, or `1`/`0`; from Scheme, pass `#t`/`#f`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `theme` | string | `""` (built-in `sand`) | Active color theme name |
| `scrolloff` | integer | `3` | Minimum lines kept above/below cursor |
| `mouse-enabled` | bool | `#t` | Enable mouse support |
| `mouse-scroll-lines` | integer | `3` | Lines per mouse scroll tick |
| `mouse-select` | bool | `#f` | Mouse drag creates selections |
| `jump-list-capacity` | integer ≥ 1 | `100` | Max jump list entries |
| `jump-line-threshold` | integer | `5` | Line distance to record a jump |
| `history-capacity` | integer ≥ 1 | `100` | Max entries per `:`/`/`/`?` prompt history |
| `undo-levels` | integer | `0` | Max undo states kept per buffer; `0` means unlimited. Once the limit is reached, the oldest states — including whole abandoned branches — are dropped as new edits are made |
| `steel-init-budget-ms` | integer ≥ 1 | `10000` | Max evaluation time (ms) for `init.scm` and each plugin activation. Setting it *from* `init.scm` has no effect on that same run — the budget is read before each file/plugin evaluation starts, so a change only takes effect for evaluations after it, i.e. the next plugin activation or the next session |
| `steel-command-budget-ms` | integer ≥ 1 | `1000` | Max Steel command evaluation time (ms) |
| `popup-border` | bool | `#t` | Show popup borders |
| `syntax-highlight-max-bytes` | integer ≥ 1 | `1048576` | Max bytes for syntax highlighting |
| `pane-dividers` | bool | `#t` | Draw a 1-cell divider between sibling panes |
| `statusline` | `left` \| `center` \| `right` | see [Statusline](#statusline) | Three `\|`-separated sections, each a comma-separated list of element names (empty sections allowed), e.g. `Mode,FileName\|\|Position` |
| `statusline.mode-colors` | bool | `#t` | Tint the whole statusline with the current mode's color; off shows the theme's base `ui.statusline` color in every mode |
| `lsp.inlay-hints` | bool | `#f` | Show inlay hints from the language server |
| `lsp.diagnostics-severity-floor` | `error` \| `warning` \| `info` \| `hint` | `hint` | Lowest diagnostic severity to display |
| `lsp.request-timeout-ms` | integer ≥ 1 | `10000` | How long to wait for a language-server reply |
| `lsp.viewport-debounce-ms` | integer ≥ 1 | `150` | Delay before re-requesting hints after scrolling |
| `wrap-mode` | `none` \| `soft[:N]` \| `word[:N]` \| `indent[:N]` | `indent` | Line wrapping for new panes. `N` is the wrap column (`0` or omitted = pane content width). See [Text wrap](#text-wrap) for per-pane overrides and the `:wrap` toggle |

## Buffer options

These options have a global default that every buffer without its own override resolves to — including buffers already open when you change it, not just ones opened afterward — and a per-buffer override that takes precedence when present. Set the global default with `:set global <option>=<value>` or `(set-option! "option" value)`; override the current buffer with `:set buffer <option>=<value>`, or from a script with `(set-buffer-option! buffer-id "option" value)` — see [Plugins](plugins.md) for setting per-language overrides from the `on-language-set` hook.

`language` is an exception, it has no global default — it is auto-detected per buffer and can only be set with `:set buffer language=<name>`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `tab-width` | integer, 1–255 | `4` | Spaces per indent level |
| `indent-guides` | bool | `#t` | Draw vertical guides at each indentation level |
| `tab-style` | `hard` \| `soft` | `hard` | What `Tab` inserts: `hard` = literal `\t`; `soft` = spaces to next tab stop |
| `line-number-style` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display in the gutter |
| `auto-pairs-enabled` | bool | `#t` | Enable auto-pair insertion |
| `select-changed-text` | bool | `#t` | After `c` (change), keeps the selection on the text you changed |
| `word-selects-whitespace` | bool | `#t` | `w`/`W`/`b`/`B` and `mm`/`MM` cover the whitespace before the destination word (trailing instead, for the first word of a line); `#f` selects the bare word instead |
| `signcolumn` | `always[:N]` \| `auto[:N]` | `always:1` | Gutter column for plugin-supplied signs (diagnostics, etc). `N` is the number of sign slots (1–127, default 1); `auto` collapses the column to zero width when no signs are visible |
| `whitespace-space` | `none` \| `all` \| `trailing` | `none` | When to render space indicators. Also reveals invisible Unicode spaces (non-breaking and ideographic) with a distinct `⍽` marker |
| `whitespace-tab` | `none` \| `all` \| `trailing` | `none` | When to render tab indicators |
| `whitespace-newline` | `none` \| `all` | `none` | When to render newline indicators |
| `language` | string | *(auto-detected)* | Language for syntax highlighting |

## Text wrap

Text wrap is controlled by a global option, a per-pane override, and a per-pane toggle command.

- `wrap-mode` is the **global option** that sets the default wrap *style* for newly opened panes. Set it in config or with `:set global wrap-mode=<value>`.
- `:set pane wrap-mode=<value>` overrides the style for the pane you're currently in, live, without affecting other panes or the global default.
- `:wrap` (alias of `:toggle-soft-wrap`) **toggles** wrapping on or off for the current pane. Turning it off disables wrapping entirely; turning it back on restores whichever style the pane was last wrapping with — whatever `:set pane wrap-mode=…` set, or otherwise the global `wrap-mode`.

`wrap-mode` is a per-pane view setting rather than a per-buffer one: two panes showing the same buffer may wrap independently — set the global default with `:set global wrap-mode=<value>` or `set-option!`, or override one pane with `:set pane wrap-mode=<value>`; there is no `:set buffer wrap-mode=...`.

Accepted values:

- `none` — no wrapping; long lines scroll horizontally.
- `soft` — break at the pane width, splitting at any character (may split a word in the middle).
- `word` — break at the pane width but prefer whitespace, so words aren't split.
- `indent` — like `word`, but continuation rows are indented to match the line's leading whitespace, so nested code stays visually nested (this is the default).
- `:N` suffix — wrap at column `N` instead of the pane's content width (e.g. `word:80`). `0` or omitted means content width.

## Themes

```scheme
(set-option! "theme" "sand")
```

To see which themes are available, type `:theme ` and press `Tab`.

Custom themes are TOML files placed in the `themes/` subdirectory of your HUME config directory. HUME uses the Helix theme format, so any theme written for Helix works in HUME too.

HUME reads these Helix statusline scopes:

- `ui.statusline` — fallback style for the statusline row, and the style
  shown in every mode when `statusline.mode-colors` is off
- `ui.statusline.normal` — row style in Normal mode
- `ui.statusline.insert` — row style in Insert mode
- `ui.statusline.separator` — separator glyph between statusline elements;
  when a theme doesn't define it, the separator matches whatever the row
  itself is currently tinted, rather than the untinted base `ui.statusline`

The whole statusline row is tinted with the current mode's color (see
`statusline.mode-colors` above); a theme that omits a mode scope falls back to
`ui.statusline`. HUME adds four more mode scopes for modes Helix doesn't have:

- `ui.statusline.extend`
- `ui.statusline.search`
- `ui.statusline.command`
- `ui.statusline.select`

Popups and menus (LSP hover, completion, the fuzzy picker) read their own scopes:

- `ui.popup` / `ui.popup.info` — hover and info popup background
- `ui.menu` / `ui.menu.selected` — completion and picker rows / the selected row
- `ui.menu.scroll` — scrollbar affordance on a scrolled popup

HUME ships a theme editor — a single-file HTML tool you can open in a browser to edit themes visually and export them as TOML. You can download it from https://github.com/cvlmtg/HUME/blob/main/tools/theme-editor/index.html

## Key bindings

```scheme
(bind-key! 'normal "ctrl-j" "move-down")
(bind-key! 'normal "g e" "goto-last-line")
(unbind-key! 'normal "ctrl-j")
```

`bind-key!` — binds a key in the given mode (`'normal`, `'insert`, `'extend`).
`unbind-key!` — removes a binding.
`bind-key-extend!` — binds a key so it always extends the selection, as the one-shot `Ctrl+` motions do.

To set several bindings at once, use the plural forms:

```scheme
(bind-keys! 'normal
  ("ctrl-h" "select-prev-word")
  ("ctrl-l" "select-next-word"))

(unbind-keys! 'normal "ctrl-j" "ctrl-k")
```

`bind-keys-extend!` is the bulk form of `bind-key-extend!`.

### Binding a key that waits for a character

Some commands need a character typed right after the key (find/till motions, surround). `bind-wait-char!` binds a key sequence so the *next* keypress is captured and passed to the target command instead of being looked up in the keymap:

```scheme
(bind-wait-char! 'normal "m s" "surround-add")
```

Inside the target command, read the captured character with `(pending-char)` — see [Plugins](plugins.md) for the full command-writing API, including the related `(request-wait-char! cmd-name)`, which waits for a character from inside an already-running command rather than from a key binding.

### Key-string grammar

A key string is a **whitespace-separated** list of tokens. Each token is `[modifier-]*key` where the modifier separator is a **dash** `-` (not `+`):

| Component | Values |
|-----------|--------|
| Modifiers | `ctrl-`, `shift-`, `alt-` (case-insensitive, repeatable, any order) |
| Named keys | `space`, `tab`, `enter` / `return` / `cr` / `ret`, `esc` / `escape`, `lt` (`<`), `backspace` / `bs`, `delete` / `del`, `insert` / `ins`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, `f1`–`f12` |
| Single char | Any single Unicode character; case is preserved (`"G"` and `"g"` are distinct) |

Multi-key sequences are space-separated: `"g e"`, `"m i w"`, `"ctrl-p h"`. Examples: `"ctrl-j"`, `"shift-tab"` (becomes `BackTab`), `"ctrl-shift-left"`, `"g e"`.

::: tip Binding the backslash key
In Scheme string literals `\` is the escape character, so to bind the `\` key write it escaped — `"\\"`, not `"\"`:

```scheme
(bind-key! 'normal "\\" "my-command")
```

The same applies to the double quote: bind `"` as `"\""`.
:::

## Statusline

The statusline is fully configurable from Steel:

```scheme
(configure-statusline! '("Mode" "Separator" "FileName") '() '("Position"))
```

Each argument is a list of element name strings: left, center, right.

Available elements:

| Element | Description |
|---------|-------------|
| `"Mode"` | Current mode label (`NOR`/`INS`/`EXT`/`CMD`/`SRC`/`SEL`) |
| `"Separator"` | Divider between sections |
| `"FileName"` | Current buffer filename (basename) |
| `"FilePath"` | Full path of current buffer |
| `"Cwd"` | Working directory |
| `"Position"` | Line and column position |
| `"Selections"` | Number of active selections |
| `"KittyProtocol"` | Kitty keyboard protocol indicator |
| `"DirtyIndicator"` | `[+]` when buffer has unsaved changes |
| `"LineEnding"` | Line ending type (LF/CRLF) |
| `"SearchMatches"` | Current search match count |
| `"MiniBuf"` | Pending key sequence hint |
| `"MacroRecording"` | Macro recording indicator |
| `"Language"` | Buffer language |
| `"ReadOnly"` | `[RO]` indicator |
| `"Diagnostics"` | Error and warning counts from the language server |

The default is equivalent to:

```scheme
(configure-statusline!
  '("Position" "FilePath" "Language" "ReadOnly" "DirtyIndicator")
  '()
  '("MacroRecording" "SearchMatches" "Diagnostics" "KittyProtocol" "Separator" "Mode"))
```

## Language detection

HUME detects file languages from extension, glob pattern, or shebang line. Define custom languages from Steel:

```scheme
(define-language! "my-lang"
  '("myl")
  '("*.my")
  '("myinterpreter"))
```

The arguments, in order, are: the language name, a list of file extensions, a list of glob patterns, and a list of shebang lines. Trailing arguments you don't need can be dropped — `(define-language! "my-lang" '("myl"))` is fine.

Write extensions **without** a leading dot: `"myl"`, not `".myl"`. An extension with a dot never matches.

The definition registers the language and associates it with tree-sitter grammars installed via PLUM:

```
:plum-install-grammar my-lang
```

See [Syntax Highlighting](syntax-highlighting.md) for the full grammar workflow — prerequisites, batch install, manual `register-grammar!`, and troubleshooting.

Hooks can trigger on language detection:

```scheme
(declare-plugin "my-plugin" #:events '(on-language-set))
```

## Example init.scm

A complete starting config — copy it to `~/.config/hume/init.scm` and edit:

```scheme
;; Bundled plugins
(load-plugin "core:stdlib")           ; helper toolkit other plugins depend on
(load-plugin "core:pickers")          ; fuzzy file/buffer finders
(declare-plugin "core:lsp")           ; language server features
(declare-plugin "core:plum")          ; plugin/grammar manager
```

Before your `init.scm` runs, HUME loads its own prelude (which defines `bind-keys!`, `define-language!` and friends) and its built-in language definitions — so those are always available to you.

## File locations

HUME resolves its directories per OS:

| Path | macOS / Linux | Windows |
|------|---------------|---------|
| Config dir (`init.scm`, user `themes/`) | `$XDG_CONFIG_HOME/hume/` (default `~/.config/hume/`) | `%APPDATA%\hume\` |
| Data dir (plugin clones, tree-sitter grammars) | `$XDG_DATA_HOME/hume/` (default `~/.local/share/hume/`) | `%LOCALAPPDATA%\hume\` (fallback `%APPDATA%\hume\`) |
| Runtime dir (bundled `runtime/`: `tutor.rst`, `themes/`, `scheme/`, `init.scm.example`, core plugins) | see below | see below |

HUME looks for its runtime directory in this order, taking the first that exists:

1. `$HUME_RUNTIME`, if set
2. `../share/hume/` relative to the binary (macOS and Linux only — this is the layout you get from the release archive)
3. `runtime/` next to the binary (the Windows archive layout)
4. `runtime/` in the current working directory (handy when running from a source checkout)

Notable subpaths inside the data dir: `data/plugins/` (PLUM-managed plugin clones), `data/grammars/` and `data/grammars/sources/` (compiled and source tree-sitter grammars).

::: warning Plugins are trusted code
Plugins run with the same privileges as HUME itself — they can read and write any file your user account can, and run other programs. There is no sandbox. Install third-party plugins only from sources you trust.
:::

::: info
On macOS HUME follows the XDG convention (`~/.config/hume/`, `~/.local/share/hume/`) rather than `~/Library/Application Support/`.
:::
