# Themes

HUME colors its interface and your code from a theme: a TOML file in the Helix theme format. Set the active one with the `theme` option (see [Configuration](configuration.md#global-options)) or `:theme <name>`.

```scheme
(set-option! "theme" "sand")
```

To see which themes are available, type `:theme ` and press `Tab`.

HUME reads `sand` from the `themes/` directories like any other theme, so editing your copy changes what you see. A built-in fallback theme is used instead when no `sand` theme is found or when the theme set in `init.scm` fails to load.

Custom themes are TOML files placed in the `themes/` subdirectory of your HUME config directory, hand-authored, alongside `init.scm`. A theme installed by a tool instead goes in the `themes/` subdirectory of your HUME data directory (see [File locations](configuration.md#file-locations)); a config-dir theme of the same name wins.

HUME reads the Helix theme format and aims to support Helix themes as they are written. It is not there in every detail yet, but it is close: most Helix themes load and render unchanged. A scope can be written as a flat key (`"ui.cursor" = { fg = "..." }`) or as a TOML section header (`[ui.cursor]` / `fg = "..."`). HUME treats the two as equivalent, though Helix itself reads only the flat form, so a section-header theme won't travel back.

A color can be a hex literal, a palette name you define, or one of the sixteen terminal color names Helix themes use (`red`, `light-gray`, and so on). These resolve to fixed colors from the standard terminal palette rather than to whatever your own terminal happens to have those colors set to, so a theme looks the same everywhere and unfocused-pane dimming has an actual color to blend toward. A color value outside these three forms leaves that one entry unstyled rather than failing the whole load, and `:messages` names it.

One thing a Helix theme can contain isn't supported, but it doesn't stop the rest of the theme from loading either: the top-level `rainbow` array. HUME has no rainbow-bracket highlighting, so it has nothing to drive. The theme still loads, and the entry is reported in `:messages` like any other one HUME couldn't use.

A theme fails to load outright only when the problem is with the document rather than one entry in it: invalid TOML syntax, an `inherits` parent that doesn't exist or forms a cycle or nests more than eight deep, or an `inherits`/`palette` key that isn't a string/table. Loading then keeps your current theme.

## Installing themes

To install a third-party theme repository, run `:plum-install-theme <user/repo>` (see [Core Plugins → core:plum](core-plugins.md#core-plum)), for example:

```
:plum-install-theme cvlmtg/everforest.hume
```

::: info
[cvlmtg/everforest.hume](https://github.com/cvlmtg/everforest.hume) is Everforest, ported from Helix: a green-based, low-contrast color scheme designed to feel warm and comfortable on the eyes, inspired by forest colors in fall.
:::

`:theme <Tab>` picks it up right away, no restart needed.

A theme editor is available online: a single-file HTML tool you download and open in a browser to edit themes visually and export them as TOML: https://raw.githubusercontent.com/cvlmtg/HUME/main/tools/theme-editor/index.html

## Theme scopes

A scope not listed below behaves as
[Helix's own theme reference](https://docs.helix-editor.com/themes.html) describes it.
Every syntax-highlighting scope works this way, so the part of a theme that colors your
code carries over as-is.

### Scopes HUME adds

These have no Helix equivalent:

- `ui.cursor.match.search`: coloring every visible search match, falling back to
  `ui.cursor.match` when unset
- `ui.popup.scroll`: scrollbar thumb on a scrolled hover popup (Helix only themes a
  scrollbar for `ui.menu`)
- `ui.window.focused`: seam divider segments adjacent to the focused pane, falling back
  to `ui.window`
- `ui.drawer`: background of the bottom drawer (`show-drawer-list!`), a generic pick-list
  panel Helix doesn't have
- `ui.tabline` / `ui.tabline.active`: the tab bar's row and its active tab. `.active` left
  unset falls back to the base `ui.tabline` style, unlike the statusline separator below
- `ui.statusline.search` / `.command` / `.sift`: one more mode-tinted statusline scope
  per HUME mode Helix doesn't have, alongside Helix's own
  `ui.statusline.normal`/`.insert`/`.select` (`.select` colors **Extend**, HUME's name for
  what Helix calls Select mode; `.sift` colors HUME's own Sift mode, the `s` regex prompt)
- `ui.virtual.invisible`: the `<200b>`-style stand-in for a character the terminal must
  not be shown as itself (see [Buffer options](configuration.md#buffer-options))
- `diff.plus.line` / `diff.minus.line` / `diff.delta.line`: the whole-line background tint
  `core:git-diff` paints for an added, deleted, or changed line (`diff.minus.line` also
  colors the ghost text of a deleted line, since nothing is left in the buffer to color).
  Falls back to nothing if left undefined. An unmodified Helix theme colors the gutter
  marker (below) but paints no line tint, which is the deliberate trade-off rather than a bug
- `diff.plus.word` / `diff.minus.word`: word-level highlight inside a changed line
  (`core:git-diff`'s inline diff), inside the line-level `.line` tint above
- `diagnostic.error.message` / `.warning.message` / `.info.message` / `.hint.message` and
  their `.message-text` counterparts: the `:messages` log's severity badge and body text,
  a HUME-only feature
- `error.diagnostic.inline` / `warning.diagnostic.inline` / `info.diagnostic.inline` /
  `hint.diagnostic.inline`: the diagnostic summary shown at the end of an offending line.
  Separate from `diagnostic.error` and friends, which style the squiggle under the code
  itself, so the summary doesn't pick up that scope's underline. Each falls back to the
  matching `error`/`warning`/`info`/`hint` gutter color when unset

### Helix scopes HUME doesn't read

Declaring any of these has no effect today. They fall into two groups, and the difference
matters if you're deciding whether to keep them in a theme you maintain.

**Waiting on a feature.** HUME doesn't have the thing these color yet. When it does, these
scopes are the natural way to theme it, so leaving them in a theme costs nothing:

- No debugger (DAP) support: `ui.debug`, `ui.debug.breakpoint`, `ui.debug.active`
- No which-key-style prompts: `ui.popup.info`, `ui.help`, `ui.text.info`
- No picker-preview highlighting: `ui.highlight`, `ui.highlight.frameline`
- No cursor-column ruler: `ui.cursorcolumn`, `ui.cursorcolumn.primary`,
  `ui.cursorcolumn.secondary`
- No per-kind completion-entry styling: `ui.text.directory`, `ui.text.symlink`
- No LSP deprecated/unnecessary diagnostic tags: `diagnostic.deprecated`,
  `diagnostic.unnecessary`
- No move- or conflict-specific diff styling: `diff.delta.moved`, `diff.delta.conflict`
- No snippet support: `tabstop`
- Ruler columns, the soft-wrap indicator, virtual jump labels, and per-kind inlay hints:
  `ui.virtual.ruler`, `ui.virtual.wrap`, `ui.virtual.jump-label`,
  `ui.virtual.inlay-hint.parameter`, `ui.virtual.inlay-hint.type`

**HUME's interface works differently.** These color a piece of Helix's UI that HUME either
doesn't present the same way or styles from another scope. They may never apply, so the
listed alternative is where to put the color instead:

- `ui.picker.header`, `ui.picker.header.column`, `ui.picker.header.column.active`: HUME's
  picker has no column headers to style
- `ui.gutter`, `ui.gutter.selected`: the gutter takes no background of its own; style it
  with `ui.linenr` and `ui.linenr.selected`
- `ui.statusline.inactive`, `ui.text.inactive`: HUME tints the whole statusline row by
  mode rather than dimming an unfocused one, and dims an unfocused pane wholesale instead
  of theming an inactive state
- `ui.cursorline.secondary`: only the primary selection's line is tinted, via
  `ui.cursorline.primary`
- `ui.background.separator`: HUME's prompt line has no separator rule beneath it
- `ui.bufferline.background`: the tab bar's ground comes from `ui.tabline` (or
  `ui.bufferline`, see below), not a separate background layer

### Scopes HUME reads differently

`ui.window` is the seam between split panes. HUME draws the divider glyph itself, so it
reads that scope's foreground; a theme that sets only a background falls back to the
theme's own base text color (`ui.text`) for it, the same fallback every other undecorated
element uses.

`ui.statusline.separator` divides the statusline's segments. HUME tints the whole
statusline row by mode, so leaving this scope undefined takes the row's own current color
rather than the untinted `ui.statusline`; otherwise the separator would show through a
mode-tinted row as a stripe of the wrong color. Set it explicitly and that wins, in every
mode.

HUME's tab bar is a saved window layout per tab (Vim's tab pages), not a per-buffer strip,
so it's styled with its own `ui.tabline` / `ui.tabline.active` scopes rather than Helix's
`ui.bufferline` / `ui.bufferline.active`. A theme that sets only the latter (every Helix
theme, since `ui.tabline` is HUME's own addition) still renders correctly: `ui.tabline`
falls back to `ui.bufferline` when unset, and `ui.tabline.active` to `ui.bufferline.active`,
so the bar picks up a ported theme's colors without it needing to name HUME's scopes at all.
