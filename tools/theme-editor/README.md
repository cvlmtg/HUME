# Helix Theme Editor

React app for building Helix-format theme TOML files, previewed live against a mock editor pane.

`index.html` at the top of this directory is **generated output** — edit files under `src/` instead, then rebuild. It's committed as a single self-contained file because the user manual links to it directly as a standalone download.

```sh
npm install
npm run dev     # local dev server with HMR
npm run build   # rebuilds tools/theme-editor/index.html
npm test        # runs tests/*.test.js against the pure-logic modules
```

## Known limitations

- Triple-quoted (multi-line) TOML strings (`"""..."""`, `'''...'''`) aren't supported by the parser.
- Scopes outside the curated list in `src/data.js` (e.g. from an imported theme) are preserved on export but aren't editable in the UI.
