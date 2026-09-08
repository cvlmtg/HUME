# HUME Theme Editor

React app for building Helix-format theme TOML files, previewed live against a mock HUME editor pane. The format is Helix's, so a theme built here loads in Helix too; the preview is HUME's, down to scopes Helix has no equivalent for.

`index.html` at the top of this directory is **generated output** — edit files under `src/` instead, then rebuild. It's committed as a single self-contained file because the user manual links to it directly as a standalone download.

```sh
npm install
npm run dev     # local dev server with HMR
npm run build   # rebuilds tools/theme-editor/index.html
npm test        # runs tests/*.test.js against the pure-logic modules
```

## Known limitations

- An 8-digit `#rrggbbaa` colour is carried through import, editing and export unchanged, but HUME's loader accepts only `#rgb` and `#rrggbb` — it rejects the alpha form and leaves that entry unstyled. The preview shows the colour anyway. Helix rejects it too, so this is a hand-edited value in either editor.
- Triple-quoted (multi-line) TOML strings (`"""..."""`, `'''...'''`) aren't supported by the parser.
- Scopes outside the catalog in `src/data.js` (e.g. from an imported theme with its own invented scopes) are preserved on export but aren't editable in the UI. The catalog is meant to track what HUME resolves, but it's maintained by hand — `tests/coverage.test.js` checks that every catalog entry outside the UI/Cursor/Virtual/Diagnostic groups is rendered somewhere in the preview and vice versa — those four groups are seeded from the catalog itself, so they pass by construction — and it never checks the catalog against HUME's own scope names.
- Exporting an `inherits` theme always writes just this theme's own overrides plus `inherits`, never a flattened standalone copy — whether the parent is still pending (the "import the parent" banner is up) or already resolved (its own content is diffed out of the export). There's no way to export a flattened, self-contained file from the UI.
- The preview can still show a different colour than HUME actually renders for a value that is neither a palette name nor one of the sixteen ANSI names `resolveColor` (`src/lib/theme.js`) knows. Such a value passes straight through to CSS, so a genuinely unsupported name (Helix accepts none beyond those sixteen, but a hand-edited theme could contain anything) previews as that CSS named colour — but HUME's loader leaves the same entry unstyled and logs a warning instead of colouring it. A top-level `rainbow` array is preserved and re-exported unchanged, since neither tool implements rainbow-bracket highlighting. Nothing in the tool surfaces either difference — check the manual's Themes section for what HUME actually does with an entry like this.
