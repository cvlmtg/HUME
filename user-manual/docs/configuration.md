# Configuration

Options, key bindings, the statusline, plugins, and language servers are all configured in one language, in one file. There's no separate config format to learn: `init.scm` is a Scheme program, so anything you can compute you can configure.

HUME can be configured two ways: the `:set` command for runtime changes during a session, or an `init.scm` file for persistent configuration loaded at startup.

HUME reads persistent configuration from:

- **macOS / Linux:** `$XDG_CONFIG_HOME/hume/init.scm` (defaults to `~/.config/hume/init.scm`)
- **Windows:** `%APPDATA%\hume\init.scm`

Pass `--config <FILE>` (see [Command-line Flags](cli.md)) to load a different file instead. [Themes](themes.md) and the data directory still resolve from the standard directories above. `:reload-config` re-runs whichever file the session started from.

If the file does not exist, HUME starts with defaults, except an explicit `--config` path, which is a startup (and reload) error if missing: unlike the default `init.scm`, an explicitly named file is expected to be there, so `:reload-config` reports an error rather than silently resetting to defaults if it's gone by the time you reload. If it fails partway through, the error is reported in `:messages` and everything up to that point stays applied, so a broken line late in the file leaves you half-configured rather than back at defaults. Fix it and run `:reload-config` to re-run the file without restarting.

A plugin that fails to load is handled more gracefully than a broken line of plain configuration: HUME reports the error, naming the plugin and pointing at the exact file and line the problem is in, and then keeps going with the rest of `init.scm`, so a broken third-party plugin near the top of your file no longer takes everything after it down with it. See [Plugins](plugins.md) for how to recover.

`:reload-config` starts from a clean slate: every option, key binding, hook, command, and plugin goes back to its default first, then the file runs again, so removing a line from `init.scm` and reloading does undo what it did. Any `:set global`/`:set buffer`/[`:theme`](themes.md) change you made during the session is discarded too, not just what `init.scm` set, with two exceptions: a pane-scoped `:set pane` override, which stays as you left it (panes are editing state, not config), and an explicit `:set buffer language=<name>`, which is restored after the reload rather than discarded: detection can't reconstruct it on its own (that's exactly why you had to set it explicitly), so losing it on every reload would be more surprising than keeping it. If the file fails partway through this time, you're left with defaults plus whatever ran before the error, same as at startup.

Buffers stay open and language servers stay attached across a reload: it behaves as if every open file were closed and reopened. Completion triggers, inline diagnostics, and any per-language setup your config applies (e.g. from `on-language-set`) come back too, without restarting the language server or losing your place in the file.

A reference config ships as `init.scm.example` inside the runtime directory (`share/hume/init.scm.example` in the macOS/Linux release archive, `runtime/init.scm.example` on Windows or in a source checkout; see [File locations](#file-locations) for the general rule). Copy it to the path above if you want a starting point, or see [Example init.scm](#example-init-scm) below.

## Setting options

There are two ways to set an option: the `:set` command for runtime changes, or `set-option!` in `init.scm` for persistent defaults.

### `:set` command

The `:set` command takes a scope and a `key=value` pair. The scope is required:

```
:set global <option>=<value>     set the global default
:set buffer <option>=<value>     override for the current buffer only (takes precedence over global)
:set pane <option>=<value>       override for the current pane only (view-scoped settings)
```

For a buffer option (the [Buffer options](#buffer-options) table below), `:set global` takes effect immediately in every buffer that has no override of its own, not just newly opened ones. `wrap-mode` additionally accepts `:set pane`, which pins one pane's wrap style above both the buffer and global setting (see [Text wrap](#text-wrap)).

Changes apply to the current session and are not persisted. For persistent configuration, use `init.scm` (below).

### From `init.scm`

```scheme
(set-option! "option-name" value)
```

Sets the global default. The value is a string, symbol, boolean, or integer. An option that takes one of a fixed set of names also accepts, and `get-option` returns, the name as a symbol: `'soft`. Callable from `init.scm`, a plugin body, or a command/hook body: anywhere Scheme code runs.

```scheme
(set-option! "line-number-style" 'absolute)
(set-option! "tab-width" 2)
```

`init.scm` is a real Scheme program, not a flat list of settings, so you can react to what's being opened rather than only set fixed defaults. The most common case is configuring an option per file type: register an `on-language-set` handler and call `(set-buffer-option! pane "option" value)` to override just that buffer (see [Hooks](plugins.md#hooks) for the full hook API):

```scheme
; 2-space indentation for Markdown buffers
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "markdown")
      (set-buffer-option! pane "tab-width" 2))))

; word-wrap Markdown buffers, leave source code unwrapped
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "markdown")
      (set-buffer-option! pane "wrap-mode" "word"))))

; treat '-' as a word character in CSS-family buffers, so `w`/`b`/`mm`/`*`
; see "foo-bar" as one word instead of three
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (member lang '("css" "scss" "less"))
      (set-buffer-option! pane "word-chars" "-"))))
```

## Global options

Set with `:set global <option>=<value>` or `(set-option! "option" value)`. All of these are global-only.

For a `bool` option, `:set` accepts `true`/`false`, `on`/`off`, `yes`/`no`, or `1`/`0`; from Scheme, pass `#t`/`#f`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `theme` | string | `sand` | Active color [theme](themes.md) name. Empty selects the built-in fallback theme |
| `cursor-shape-insert` | `block`/`bar`/`underline` | `bar` | Cursor shape while in Insert mode, applied to every cursor when multiple are active |
| `scroll-margin` | integer | `3` | Minimum lines kept above/below cursor |
| `object-jump-align` | `top`/`center`/`off` | `center` | Where the view lands after jumping forward to a paragraph or structural object (`}`, `g f`, …); `top` is still subject to `scroll-margin` |
| `mouse` | bool | `#t` | Enable mouse support |
| `mouse-scroll-lines` | integer | `3` | Lines per mouse scroll tick |
| `mouse-select` | bool | `#f` | Mouse drag creates selections |
| `jump-list-capacity` | integer ≥ 1 | `100` | Max jump list entries |
| `jump-line-threshold` | integer | `5` | Line distance to record a jump |
| `history-capacity` | integer ≥ 1 | `100` | Max entries per `:`/`/`/`?` prompt history |
| `undo-levels` | integer | `0` | Max undo states kept per buffer; `0` means unlimited. Once the limit is reached, the oldest states (including whole abandoned branches) are dropped as new edits are made |
| `steel-init-budget-ms` | integer ≥ 1 | `10000` | Max evaluation time (ms) for `init.scm` and each plugin activation. Setting it *from* `init.scm` has no effect on that same run: the budget is read before each file/plugin evaluation starts, so a change only takes effect for evaluations after it, i.e. the next plugin activation or the next session |
| `steel-command-budget-ms` | integer ≥ 1 | `1000` | Max Steel command evaluation time (ms) |
| `command-completion` | `common-prefix` \| `first-candidate` | `common-prefix` | What the first `Tab` on the `:` line does when several candidates match: `common-prefix` completes the longest text they share and picks nothing, so the next `Tab` picks the first candidate; `first-candidate` fills in the first candidate at once |
| `popup-border` | bool | `#t` | Show popup borders |
| `syntax-highlight-max-bytes` | integer ≥ 1 | `1048576` | Max bytes for syntax highlighting |
| `pane-dividers` | bool | `#t` | Draw a 1-cell divider between sibling panes |
| `statusline` | `left` \| `center` \| `right` | see [Statusline](#statusline) | Three `\|`-separated sections of comma-separated element names. Set it with `configure-statusline!` |
| `statusline.mode-colors` | bool | `#t` | Tint the whole statusline with the current mode's color; off shows the [theme](themes.md)'s base `ui.statusline` color in every mode |
| `tabline` | `always`/`never`/`dynamic` | `dynamic` | When to show the tab bar: always, never, or only once more than one tab is open |

The `lsp.*` options below configure `core:lsp`. See [Language Servers](lsp.md) for setup, commands, and how they're used.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `lsp.inlay-hints` | bool | `#f` | Show inferred types and parameter names inline, next to the code they describe |
| `lsp.diagnostics-severity-floor` | `error` \| `warning` \| `info` \| `hint` | `hint` | Lowest diagnostic severity to display |
| `lsp.diagnostics-on-insert-line` | bool | `#f` | Keep the diagnostic underline and end-of-line summary on the line you are typing on in Insert mode. Off hides them there |
| `lsp.request-timeout-ms` | integer ≥ 1 | `10000` | How long to wait for a language-server reply |
| `lsp.viewport-debounce-ms` | integer ≥ 1 | `150` | Delay before re-requesting hints after scrolling |
| `lsp.diagnostics-pull-debounce-ms` | integer ≥ 1 | `250` | Delay after you stop typing before asking a server that only reports on request for its diagnostics |
| `lsp.format-max-ranges` | integer ≥ 1 | `16` | Above this many disjoint ranges, `:lsp-fmt` warns and formats nothing instead of sending one request per range (a server that batches ranges into a single request isn't capped) |

## Buffer options

These options have a global default that every buffer without its own override resolves to (including buffers already open when you change it, not just ones opened afterward) and a per-buffer override that takes precedence when present. Set the global default with `:set global <option>=<value>` or `(set-option! "option" value)`; override the current buffer with `:set buffer <option>=<value>`, or from a script with `(set-buffer-option! pane "option" value)`. See [Plugins](plugins.md) for setting per-language overrides from the `on-language-set` hook.

`language` and `line-ending` are exceptions, they have no global default. `language` is auto-detected per buffer, and setting it overrides the detection for that buffer. `:set buffer language=` with nothing after the `=` (or `""` from a script) clears it. `line-ending` is read from the file when it opens (new buffers use `lf`) and can only be set per buffer.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `wrap-mode` | `none` \| `soft[:N]` \| `word[:N]` \| `indent[:N]` | `indent` | Line wrapping. `N` is the wrap column (`0` or omitted = pane content width). Also accepts `:set pane wrap-mode=<value>` to pin one pane above the buffer/global setting; see [Text wrap](#text-wrap) |
| `tab-width` | integer, 1–255 | `4` | Spaces per indent level |
| `indent-guides` | bool | `#t` | Draw vertical guides at each indentation level |
| `tab-style` | `hard` \| `soft` | `hard` | What `Tab` inserts: `hard` = literal `\t`; `soft` = spaces to next tab stop |
| `line-number-style` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display in the gutter |
| `auto-pairs` | bool | `#t` | Enable auto-pair insertion |
| `select-inserted-text` | bool | `#t` | Leaving Insert mode keeps the text you typed selected, instead of leaving a plain cursor |
| `word-selects-whitespace` | bool | `#t` | `w`/`W`/`b`/`B` and `mm`/`MM` cover the whitespace before the destination word (the trailing whitespace instead when none precedes it: the first word of a line, or a word directly after punctuation); `#f` selects the bare word instead |
| `word-chars` | string | `""` | Extra characters counted as part of a word by `w`/`b`, `mm`, `miw`/`maw`, `select-word-nearest-on-line`, `Ctrl-w`, and `*`, e.g. `-` makes `foo-bar` one word instead of three. Also affects quote auto-pairing (`'`, `"`, `` ` ``, not bracket pairs), the identifier under the cursor used by plugin commands (e.g. rename), and where a completion without a server-supplied replace range starts. Does not affect `W`/`B`/`MM`, which already treat punctuation as part of a WORD. No global default ships; set it per language from an `on-language-set` hook (see below). Whitespace and newline characters are rejected |
| `signcolumn` | `always[:N]` \| `auto[:N]` | `always` | Gutter column for plugin-supplied signs (diagnostics, git changes, etc). Bare `always`/`auto` sizes the column to one column per registered sign source: a source claims its column the moment the plugin registers it, so the width doesn't change as individual signs come and go; `:N` pins it to exactly N columns (1–127) instead, hiding whichever lower-priority sources don't fit. `auto` additionally collapses to zero width when no signs are visible |
| `auto-read` | bool | `#t` | Prompt to reload when the current buffer's file changes on disk. `#f` only warns; reload manually with `:e!` |
| `whitespace-space` | `none` \| `all` \| `trailing` | `none` | When to render space indicators. Also reveals invisible Unicode spaces (non-breaking and ideographic) with a distinct `⍽` marker |
| `whitespace-tab` | `none` \| `all` \| `trailing` | `none` | When to render tab indicators |
| `whitespace-newline` | `none` \| `all` | `none` | When to render newline indicators |
| `language` | string | *(auto-detected)* | The buffer's language: picks its syntax highlighting, language server, and language-specific plugins |
| `line-ending` | `lf` \| `crlf` | *(from the file)* | The line ending a save writes. Changing it marks the buffer modified but is not an undo step, so `u` does not revert it; set it back to the file's own ending and the buffer reads unmodified again |

Characters the terminal cannot be shown (control characters, and invisible ones such as a zero-width space or a bidirectional override) are always displayed as their codepoint (`<200b>`), styled with the [theme](themes.md#scopes-hume-adds)'s `ui.virtual.invisible` scope, whatever the options above are set to. They are not whitespace you can choose to hide: left invisible they misalign the rest of the line, and an unseen bidirectional override can make code read differently from how it runs.

## Text wrap

Text wrap is controlled by three layers: a global default, a per-buffer override, and a per-pane pin, plus a per-pane toggle command.

- `wrap-mode` is a **buffer option** (see [Buffer options](#buffer-options)): set a global default with `:set global wrap-mode=<value>` or `set-option!`, or override one buffer with `:set buffer wrap-mode=<value>` or `(set-buffer-option! pane "wrap-mode" value)`. To wrap by file type (markdown but not source code, say), set it per language from an `on-language-set` hook (see [Plugins](plugins.md)).
- `:set pane wrap-mode=<value>` pins the style for the pane you're currently in and the buffer it's currently showing, live, above both the buffer and global setting, without affecting other panes on the same buffer. Switching that pane to a different buffer resolves the new buffer's own setting instead; switching back returns the pin. There's no command to clear a pin back to following the buffer/global setting. Pin it to a different value, or close the buffer, to move on from it.
- `:wrap` (alias of `:toggle-soft-wrap`) **toggles** wrapping on or off for the current pane and buffer. Turning it off pins the pane to no wrap; turning it back on restores whatever it was doing before: the buffer/global setting, if the pane wasn't pinned, or the exact style you pinned it to with `:set pane wrap-mode=…`. If that restores a setting that doesn't actually wrap, it pins the configured global style instead (or `indent`, if the global itself is `none`), so `:wrap` always visibly wraps.

Two panes showing the same buffer can still wrap independently once one of them is pinned with `:set pane` or `:wrap`; that's what the per-pane layer is for.

Accepted values:

- `none`: no wrapping; long lines scroll horizontally.
- `soft`: break at the pane width, splitting at any character (may split a word in the middle).
- `word`: break at the pane width but prefer whitespace, so words aren't split.
- `indent`: like `word`, but wrapped continuation lines are indented to match the line's leading whitespace, so nested code stays visually nested (this is the default).
- `:N` suffix: wrap at column `N` instead of the pane's content width (e.g. `word:80`). `0` or omitted means content width.

## Key bindings

```scheme
(bind-key! 'normal "ctrl-j" "move-down")
(bind-key! 'normal "g e" "goto-last-line")
(unbind-key! 'normal "ctrl-j")
```

`bind-key!` takes an editor command's name (the same names in [Builtin Commands](builtin-commands.md)), never a typed command; there's no way to bind one of those to a key.

`bind-key!`: binds a key in the given mode (`'normal`, `'insert`, `'extend`).
`unbind-key!`: removes a binding.
`bind-key-extend!`: binds a key so it always extends the selection, as the one-shot `Ctrl-` motions do.

To set several bindings at once, use the plural forms:

```scheme
(bind-keys! 'normal
  ("ctrl-h" "select-prev-word")
  ("ctrl-l" "select-next-word"))

(bind-keys-extend! 'normal
  ("ctrl-n" "select-line")
  ("ctrl-y" "select-line-backward"))

(unbind-keys! 'normal "ctrl-j" "ctrl-k")
```

`bind-keys!` batches `bind-key!`, `bind-keys-extend!` batches `bind-key-extend!`, and `unbind-keys!` batches `unbind-key!`. Each takes one or more `(key cmd)` pairs (or, for `unbind-keys!`, one or more bare keys) instead of a single one.

### Falling back to a key's normal Insert-mode behaviour

Binding an Insert-mode key replaces its behaviour outright. `insert-key!` (see [Plugin API](plugin-api.md#editing-navigation), which also has a worked example) lets a command decide, per keypress, whether to override the key or fall back to what it would otherwise do.

`.` repeats an Insert-mode key's binding by re-running it, not by replaying a fixed effect, so a binding decides again at the new cursor, taking whichever branch fits there. The one exception is a branch that asks you for input while it runs: opening the completion popup, accepting a completion, or opening a picker are never re-run: `.` writes what they produced instead, and reports an error for that keypress if the binding would take that branch again on replay. A branch with no such step, like one that saves the file, runs again in full, including that side effect.

### Binding a key that waits for a character

Some commands need a character typed right after the key (find/till motions, surround). `bind-wait-char!` binds a key sequence so the *next* keypress is captured and passed to the target command instead of being looked up in the keymap:

```scheme
(bind-wait-char! 'normal "m s" "surround-add")
```

Inside the target command, read the captured character with `(pending-char)`; see [Plugins](plugins.md) for the full command-writing API, including the related `(request-wait-char! cmd-name)`, which waits for a character from inside an already-running command rather than from a key binding.

### Key-string grammar

A key string is a **whitespace-separated** list of tokens. Each token is `[modifier-]*key` where the modifier separator is a **dash** `-` (not `+`):

| Component | Values |
|-----------|--------|
| Modifiers | `ctrl-`/`c-`, `shift-`/`s-`, `alt-`/`a-` (case-insensitive, repeatable, any order, short and long forms may be mixed) |
| Named keys | `space`, `tab`, `enter` / `return` / `cr` / `ret`, `esc` / `escape`, `lt` (`<`), `backspace` / `bs`, `delete` / `del`, `insert` / `ins`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, `f1`–`f12` |
| Single char | Any single Unicode character; case is preserved (`"G"` and `"g"` are distinct); an uppercase letter and an explicit `shift-` are equivalent (`"ctrl-X"` and `"ctrl-shift-x"` bind the same key) |

Multi-key sequences are space-separated: `"g e"`, `"m i w"`, `"ctrl-p h"`. Examples: `"ctrl-j"`, `"shift-tab"` (becomes `BackTab`), `"ctrl-shift-left"`, `"g e"`.

::: tip Binding the backslash key
In Scheme string literals `\` is the escape character, so to bind the `\` key write it escaped: `"\\"`, not `"\"`:

```scheme
(bind-key! 'normal "\\" "my-command")
```

The same applies to the double quote: bind `"` as `"\""`.
:::

## Statusline

The statusline is fully configurable from Scheme:

```scheme
(configure-statusline! '("Mode" "Separator" "FileName") '() '("Position"))
```

Each argument is a list of element name strings: left, center, right.

Available elements:

| Element | Description |
|---------|-------------|
| `"Mode"` | Current mode label (`NOR`/`INS`/`EXT`/`CMD`/`SRC`/`SIF`) |
| `"Separator"` | Divider between sections |
| `"FileName"` | Current buffer filename (basename) |
| `"FilePath"` | Full path of current buffer |
| `"Cwd"` | Working directory |
| `"Position"` | Line and column position; column counts graphemes (`h`/`l` presses), matching `:diagnostics` and goto/references lists |
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

### Custom elements

Place `"steel:<name>"` for any `<name>` of your choosing to add your own element. `<name>` must be non-empty and must *not* contain `,` or `|`. Push its text with `set-statusline-text!`, driven by whatever should trigger an update (a hook, a timer, a command):

```scheme
(configure-statusline! '("steel:line-count" "FilePath") '() '("Position" "Mode"))

(define (refresh-line-count! pane)
  (set-statusline-text! "line-count" pane
    (string-append (number->string (buffer-line-count pane)) "L")))

(register-hook! 'on-text-changed refresh-line-count!)
(register-hook! 'on-buffer-enter refresh-line-count!)
```

`core:git-diff` ships a `"steel:git-branch"` element using this same mechanism (see [Core Plugins → core:git-diff](core-plugins.md#core-git-diff)); just add it to your own `configure-statusline!` call.

`set-statusline-text!` takes the element name, a pane, and the text to show; an empty string clears it. Each buffer keeps its own value per name, and a placed element shows only the focused buffer's: switching to a buffer with nothing pushed yet shows nothing, same as any other element with no content. Placing the element and pushing its text are independent: either can happen first, and neither errors if the other hasn't happened yet.

## Language detection

HUME detects file languages from extension, glob pattern, or shebang line. See [Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language) for defining custom languages with `define-language!` and, for grammars outside the catalog, `register-grammar!`.

## Example init.scm

A complete starting config. Copy it to `~/.config/hume/init.scm` and edit:

```scheme
;; Bundled plugins
(load-plugin! "core:stdlib")           ; helper toolkit other plugins depend on
(load-plugin! "core:pickers")          ; fuzzy file/buffer finders
(load-plugin! "core:lsp")              ; language server features
(load-plugin! "core:lsp-install")      ; :lsp-install / :lsp-uninstall for language servers
(load-plugin! "core:plum")             ; plugin/grammar manager
(load-plugin! "core:git-diff")         ; live inline git diff
```

Before your `init.scm` runs, HUME loads its own prelude (which defines `bind-keys!`, `define-language!` and friends) and its built-in language definitions, so those are always available to you.

## File locations

HUME resolves its directories per OS:

| Path | macOS / Linux | Windows |
|------|---------------|---------|
| Config dir (`init.scm`, hand-authored `themes/`) | `$XDG_CONFIG_HOME/hume/` (default `~/.config/hume/`) | `%APPDATA%\hume\` |
| Data dir (plugin clones, tree-sitter grammars, installed `themes/`) | `$XDG_DATA_HOME/hume/` (default `~/.local/share/hume/`) | `%LOCALAPPDATA%\hume\` (fallback `%APPDATA%\hume\`) |
| Runtime dir (bundled `runtime/`: `tutor.rst`, `themes/`, `scheme/`, `init.scm.example`, core plugins) | see below | see below |

`--config <FILE>` overrides only which file HUME evaluates as `init.scm`; user `themes/` and the data dir still resolve from the config dir above regardless.

HUME looks for its runtime directory in this order, taking the first that exists:

1. `$HUME_RUNTIME`, if set
2. `../share/hume/` relative to the binary (macOS and Linux only; this is the layout you get from the release archive)
3. `runtime/` next to the binary (the Windows archive layout)
4. `runtime/` in the current working directory (handy when running from a source checkout)

A relative value in `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`, `$HUME_RUNTIME` or `$TMPDIR` is resolved against the directory HUME was started in, once, at startup. A later `:cd` does not change where these point.

Notable subpaths inside the data dir: `data/plugins/` (PLUM-managed plugin clones), `data/grammars/` and `data/grammars/sources/` (compiled and source tree-sitter grammars), `data/themes/` (installed third-party themes).

::: warning Plugins are trusted code
Plugins run with the same privileges as HUME itself: they can read and write any file your user account can, and run other programs. There is no sandbox. Install third-party plugins only from sources you trust.
:::

::: info
On macOS HUME follows the XDG convention (`~/.config/hume/`, `~/.local/share/hume/`) rather than `~/Library/Application Support/`.
:::
