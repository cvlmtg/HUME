# Settings

Every option HUME has, how text wrap works, the statusline elements, and how a file's language is detected. See [Setting options](configuration.md#setting-options) for how to change them.

An option's name starts with the area it belongs to: `indent-`, `insert-`, `gutter-`, `lsp-`, and so on. Type the area after `:set global ` and press `Tab` to list every option in it.

## Global options

Set with `:set global <option>=<value>` or `(set-option! "option" value)`. All of these are global-only.

For a `bool` option, `:set` accepts `true`/`false`, `on`/`off`, `yes`/`no`, or `1`/`0`; from Scheme, pass `#t`/`#f`. The tables write defaults as `true`/`false`.

The `lsp-` options apply to every language server. `core:lsp` takes its own options when you load it; see [Language Servers](lsp.md#settings) for those, and for setup and commands.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `command-completion` | `common-prefix` \| `first-candidate` | `common-prefix` | What the first `Tab` at the `:` or `\|` prompt does when several candidates match. `common-prefix` completes their longest shared text without picking one, so the next `Tab` picks the first; `first-candidate` fills in the first candidate at once |
| `completion-docs` | bool | `true` | Show the highlighted completion's details and documentation in a box beside the completion menu. When off, or when the box doesn't fit beside the menu, the details show in the menu instead. |
| `hook-before-save-timeout-ms` | integer ≥ 1 | `1000` | How long a save waits for `on-before-buffer-save` handlers before writing the file anyway |
| `hook-viewport-debounce-ms` | integer ≥ 1 | `150` | Delay after scrolling stops before plugins are told which lines are visible, e.g. to request inlay hints for them |
| `insert-cursor-shape` | `block` \| `bar` \| `underline` | `bar` | Cursor shape in Insert mode, for every cursor when there are several |
| `jump-list-capacity` | integer ≥ 1 | `100` | Max jump list entries |
| `jump-list-min-lines` | integer | `5` | Line distance to record a jump |
| `lsp-diagnostics-min-severity` | `error` \| `warning` \| `info` \| `hint` | `hint` | Lowest diagnostic severity to display |
| `lsp-diagnostics-on-insert-line` | bool | `false` | Keep the diagnostic underline and end-of-line summary on the line you are typing on in Insert mode. Off hides them there |
| `lsp-diagnostics-pull-debounce-ms` | integer ≥ 1 | `250` | Delay after you stop typing before asking a server that only reports on request for its diagnostics |
| `lsp-request-timeout-ms` | integer ≥ 1 | `10000` | How long to wait for a language-server reply |
| `mouse` | bool | `true` | Enable mouse support |
| `mouse-scroll-lines` | integer | `3` | Lines per mouse scroll tick |
| `mouse-select` | bool | `false` | Mouse drag creates selections |
| `pane-dividers` | bool | `true` | Draw a 1-cell divider between sibling panes |
| `popup-border` | bool | `true` | Show borders on popups, menus, pickers and the bottom drawer |
| `prompt-history-levels` | integer ≥ 1 | `100` | Max entries per `:`/`/`/`?`/`\|` prompt history |
| `scroll-jump-align` | `top` \| `center` \| `off` | `center` | Where the view lands after jumping forward to a paragraph or structural object (`}`, `g f`, …); `top` still respects `scroll-margin` |
| `scroll-margin` | integer | `3` | Minimum lines kept above/below cursor |
| `shell` | word list | `sh -c` | The program and leading arguments that run a shell command typed after `\|`. The command is passed as the last argument. Separate words with spaces; put a word that contains spaces in double quotes. From Scheme it is a list of strings: `(set-option! "shell" '("fish" "-c"))`. `Tab` on the `\|` prompt follows this shell's quoting (see [Shell commands](editing.md#shell-commands)) |
| `statusline` | `left` \| `center` \| `right` | see [Statusline](#statusline) | Three `\|`-separated sections of comma-separated element names. Set it with `configure-statusline!`. From Scheme, `get-option` returns it as `(hash 'left 'center 'right)`, each a list of element names, and `set-option!` takes that hash |
| `statusline-mode-colors` | bool | `true` | Tint the whole statusline with the current mode's color; off shows the [theme](themes.md)'s base `ui.statusline` color in every mode |
| `steel-command-budget-ms` | integer ≥ 1 | `1000` | Max time (ms) a script command, hook or callback may spend running Scheme before it is stopped. Time spent waiting on the system for it, such as for a program it runs, doesn't count. A stopped script runs none of its error handlers |
| `steel-init-budget-ms` | integer ≥ 1 | `10000` | The same budget for `init.scm` and for loading a plugin. A plugin that a `call!` inside a running command loads uses `steel-command-budget-ms` instead. Set from `init.scm`, it applies from the next plugin load |
| `syntax-highlight-max-bytes` | integer ≥ 1 | `1048576` | Files larger than this many bytes are not highlighted |
| `tabline` | `always` \| `never` \| `dynamic` | `dynamic` | When to show the tab bar: always, never, or only once more than one tab is open |
| `theme` | string | `sand` | Active color [theme](themes.md) name. Empty selects the built-in fallback theme |
| `undo-levels` | integer | `0` | Max undo states kept per buffer; `0` means unlimited. Past the limit, new edits drop the oldest states, including whole abandoned branches |

## Buffer options

Each of these options, except `language` and `line-ending`, has a global default and a per-buffer override, which wins when present. A buffer without an override follows the global default, including buffers already open when you change it.

- Set the global default with `:set global <option>=<value>` or `(set-option! "option" value)`.
- Override the current buffer with `:set buffer <option>=<value>`, or from a script with `(set-buffer-option! pane "option" value)`.

See [From `init.scm`](configuration.md#from-init-scm) for setting per-language overrides from the `on-language-set` hook.

`language` and `line-ending` have no global default:

- `language` is auto-detected per buffer, and setting it overrides the detection for that buffer. `:set buffer language=` with nothing after the `=` (or `""` from a script) clears it.
- `line-ending` is read from the file when it opens (new buffers use `lf`) and can only be set per buffer. Reloading the file from disk takes its line ending again.

A buffer option you set, with `:set buffer` or from a hook in your `init.scm`, beats a value a plugin sets for the same buffer, whichever comes first. `language` and `line-ending` rank the same way.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `comment-prefer-line` | bool | `false` | `G G` comments a selection spanning several lines with a line comment on each line, even when the language also has block comments |
| `file-reload-prompt` | bool | `true` | Prompt to reload when the current buffer's file changes on disk. `false` only warns; reload manually with `:e!` |
| `gutter-line-numbers` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display in the gutter |
| `gutter-signs` | `always[:N]` \| `auto[:N]` | `always` | Gutter column for plugin-supplied signs (diagnostics, git changes, etc). Bare `always`/`auto` gives one column per registered sign source. A source claims its column when the plugin registers it, so the width doesn't change as signs come and go. `:N` fixes the width at N columns (1–127) instead, hiding lower-priority sources that don't fit. `auto` also collapses to zero width when no signs are visible |
| `indent-guides` | bool | `true` | Draw vertical guides at each indentation level |
| `indent-smart` | bool | `true` | Enter, `o` and `O` indent a new line from the language's syntax, or copy the current line's indent when the language has no grammar. A closing token such as `}` typed on that new line moves it back to its block's indent. `false`: new lines start at the left margin, and nothing moves back as you type |
| `indent-style` | `hard` \| `soft` | `hard` | What `Tab` inserts: `hard` = literal `\t`; `soft` = spaces to next tab stop |
| `indent-width` | integer, 1–255 | `4` | Spaces per indent level, and how wide a tab character is drawn |
| `insert-auto-pairs` | bool | `true` | Enable auto-pair insertion |
| `insert-select-on-exit` | bool | `true` | Leaving Insert mode keeps the text you typed selected, instead of leaving a plain cursor |
| `language` | string | *(auto-detected)* | The buffer's language: picks its syntax highlighting, language server, and language-specific plugins |
| `line-ending` | `lf` \| `crlf` | *(from the file)* | The line ending a save writes. Changing it marks the buffer modified but is not an undo step, so `u` does not revert it. Setting it back to the file's own ending makes the buffer unmodified again |
| `whitespace-newline` | `none` \| `all` | `none` | When to render newline indicators |
| `whitespace-space` | `none` \| `all` \| `trailing` | `none` | When to render space indicators. Also reveals invisible Unicode spaces (non-breaking and ideographic) with a distinct `⍽` marker |
| `whitespace-tab` | `none` \| `all` \| `trailing` | `none` | When to render tab indicators |
| `word-chars` | string | `""` | Extra characters counted as part of a word, e.g. `-` makes `foo-bar` one word instead of three. Set it per language from an `on-language-set` hook (see [From `init.scm`](configuration.md#from-init-scm)). Everything it affects is listed under this table |
| `word-selects-whitespace` | `off` \| `before` \| `after` | `before` | Which whitespace `w`/`W`/`b`/`B`, `m m`/`M M` and `m a w`/`m a W` add to the word they select. Listed under this table |
| `wrap-mode` | `none` \| `soft[:N]` \| `word[:N]` \| `indent[:N]` | `indent` | Line wrapping. `N` is the wrap column (`0` or omitted = pane content width). `:set pane wrap-mode=<value>` also pins one pane above the buffer/global setting; see [Text wrap](#text-wrap) |

::: details What `word-selects-whitespace` selects
- `before`: the whitespace before the word, or after it when none precedes it (the first word of a line, or a word right after punctuation).
- `after`: the whitespace after the word, up to the end of the line, or before it when none follows (the last word of a line, or a word right before punctuation).
- `off`: the bare word; `m a w`/`m a W` then select the same as `m i w`/`m i W`.

Neither `before` nor `after` ever selects a line's indentation.
:::

::: details What `word-chars` affects
The characters count as part of a word for `w`/`b`, `m m`, `m i w`/`m a w`, the `select-word-nearest-on-line` command, `Ctrl-w`, and `*`. They also affect:

- quote auto-pairing (`'`, `"`, `` ` ``, not bracket pairs)
- the identifier under the cursor that plugin commands use (e.g. rename)
- where a completion without a server-supplied replace range starts
- when a closing keyword typed on a new line counts as complete, so it moves back to its block's indent (see `indent-smart`)
- whether `Ctrl-a`/`Ctrl-s` read a `-` as a minus sign

They don't affect `W`/`B`/`M M`, which already treat punctuation as part of an uppercase word. Whitespace and newline characters are rejected.
:::

Characters the terminal cannot show (control characters, and invisible ones such as a zero-width space or a bidirectional override) always display as their codepoint (`<200b>`), styled with the [theme](themes.md#scopes-hume-adds)'s `ui.virtual.invisible` scope, whatever the options above say. You can't hide them as you can whitespace.

## Text wrap

Three layers control text wrap: a global default, a per-buffer override and a per-pane pin. A command also toggles it per pane.

- `wrap-mode` is a **buffer option** (see [Buffer options](#buffer-options)). Set a global default with `:set global wrap-mode=<value>` or `set-option!`, or override one buffer with `:set buffer wrap-mode=<value>` or `(set-buffer-option! pane "wrap-mode" value)`. To wrap by file type (markdown but not source code, say), set it per language from an `on-language-set` hook (see [From `init.scm`](configuration.md#from-init-scm)).
- `:set pane wrap-mode=<value>` pins the style for the current pane and the buffer it's showing, above both the buffer and global setting, without affecting other panes on the same buffer.
- `:wrap` (alias of `:toggle-soft-wrap`) **toggles** wrapping on or off for the current pane and buffer.

::: details How pins and `:wrap` work together
Switching a pinned pane to a different buffer uses the new buffer's own setting; switching back returns the pin. No command makes a pinned pane follow the buffer/global setting again. To move on from a pin, pin a different value or close the buffer.

Turning wrapping off with `:wrap` pins the pane to no wrap. Turning it back on restores what the pane did before: the buffer/global setting if the pane wasn't pinned, or the style you pinned with `:set pane wrap-mode=…`. If that setting doesn't wrap, `:wrap` pins the configured global style instead (or `indent`, if the global itself is `none`), so turning it on always visibly wraps.

Two panes showing the same buffer wrap independently once one of them is pinned with `:set pane` or `:wrap`.
:::

Accepted values:

- `none`: no wrapping; long lines scroll horizontally.
- `soft`: break at the pane width, splitting at any character (may split a word in the middle).
- `word`: break at the pane width but prefer whitespace, so words aren't split.
- `indent`: like `word`, but continuation lines are indented to match the line's leading whitespace, so nested code stays visually nested. This is the default.
- `:N` suffix: wrap at column `N` instead of the pane's content width (e.g. `word:80`). `0` or omitted means content width.

## Statusline

The statusline is configured from Scheme:

```scheme
(configure-statusline! '("Mode" "Separator" "FileName") '() '("Position"))
```

The three arguments are lists of element name strings for the left, center and right sections.

Available elements:

| Element | Description |
|---------|-------------|
| `"Mode"` | Current mode label (`NOR`/`INS`/`EXT`/`CMD`/`SRC`/`SIF`) |
| `"Separator"` | Divider between sections |
| `"FileName"` | Current buffer filename (basename) |
| `"FilePath"` | Full path of current buffer |
| `"Cwd"` | Working directory |
| `"Position"` | Line and column; the column counts graphemes (`h`/`l` presses), matching `:diagnostics` and goto/references lists |
| `"KittyProtocol"` | Kitty keyboard protocol indicator |
| `"DirtyIndicator"` | `[+]` when buffer has unsaved changes |
| `"LineEnding"` | Line ending type (LF/CRLF) |
| `"SearchMatches"` | Current search match count, hidden once you type in Insert mode until you leave it |
| `"MiniBuf"` | The open prompt and the text typed into it |
| `"MacroRecording"` | Macro recording indicator |
| `"Pipe"` | The shell command running after `\|`, and how many of its processes have finished |
| `"Language"` | Buffer language |
| `"ReadOnly"` | `[RO]` indicator |
| `"Diagnostics"` | Error and warning counts from the language server, at or above `lsp-diagnostics-min-severity` |

The default is equivalent to:

```scheme
(configure-statusline!
  '("Position" "FilePath" "Language" "ReadOnly" "DirtyIndicator")
  '()
  '("Pipe" "MacroRecording" "SearchMatches" "Diagnostics" "KittyProtocol" "Separator" "Mode"))
```

### Custom elements

To add your own element, place `"steel:<name>"` with any `<name>` you choose. `<name>` must be non-empty and must not contain `,` or `|`. Push its text with `set-statusline-text!` from whatever should update it (a hook, a timer, a command):

```scheme
(configure-statusline! '("steel:line-count" "FilePath") '() '("Position" "Mode"))

(define (refresh-line-count! pane)
  (set-statusline-text! pane "line-count"
    (string-append (number->string (buffer-line-count pane)) "L")))

(register-hook! 'on-text-changed (lambda (pane changes) (refresh-line-count! pane)))
(register-hook! 'on-buffer-enter refresh-line-count!)
```

`core:git-diff` ships a `"steel:git-branch"` element built this way (see [Core Plugins → core:git-diff](core-plugins.md#core-git-diff)); add it to your own `configure-statusline!` call.

`set-statusline-text!` takes a pane, the element name, and the text to show; an empty string clears it. Each buffer keeps its own value per name, and a placed element shows only the focused buffer's. A buffer with nothing pushed yet shows nothing there, like any other element with no content. Placing the element and pushing its text are independent: either can come first, and neither errors if the other hasn't happened yet.

## Language detection

HUME detects a file's language from its extension, a glob pattern, or its shebang line. See [Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language) for defining custom languages with `define-language!`, setting their project roots and language-server identifier with `configure-language!` and, for grammars HUME doesn't ship, `register-grammar!`.
