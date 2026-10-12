# Sync scripts

Dev-time scripts that regenerate the checked-in seeded data files in `runtime/scheme/` and
`runtime/plugins/core/lsp-install/data/` from pinned upstream revisions. One script per pin; the editor never parses upstream
formats at runtime. Design rationale for the LSP parts: `docs/LSP-INSTALL.md`.

| Script | Pin | Fetches | Emits |
|---|---|---|---|
| `sync-grammars.py` | `runtime/scheme/helix-pin.scm` | `helix-editor/helix` `languages.toml` | `runtime/scheme/languages.scm`, one `runtime/scheme/languages/<name>.scm` per language, one `runtime/plugins/core/lsp-install/data/registrations/<server>.scm` per server, and `data/language-servers.scm` |
| `sync-lsp-sources.py` | `runtime/plugins/core/lsp-install/data/mason-pin.scm` | `mason-org/mason-registry` `registry.json.zip` + every release asset (for sha256) | one `runtime/plugins/core/lsp-install/data/sources/<server>.scm` and one `data/requirements/<server>.scm` per server |

Each one-file-per-name directory (`runtime/scheme/languages/`, and `registrations/`,
`sources/` and `requirements/` under `lsp-install/data/`) is owned by the script that writes
it: the script deletes the file of every name it does not emit, and stops with an error
if the directory holds anything other than `.scm` files, so nothing hand-written belongs
there. Language and server names must match `FILE_NAME` in `sync-grammars.py`, so each stays
distinct on a case-insensitive file system.

Shared helpers (pin reading, sexpr emission, atomic writes, the one-file-per-name writer and
reader) live in `sync_common.py`. The
parsing logic is tested against fixtures, without the network, by
`python3 -m unittest discover -s scripts/tests`.

## Run order

Each script runs alone after its own pin bump:

- bump `helix-pin.scm` → run `sync-grammars.py`
- bump `lsp-install/data/mason-pin.scm` → run `sync-lsp-sources.py`

One exception: `sync-lsp-sources.py` reads the checked-in `registrations/` to filter
Mason to the servers Helix actually wires — through an explicit Helix→Mason name-mapping
table (the namespaces differ: Helix `pylsp` is Mason `python-lsp-server`), reporting every
Helix server left unmatched. So after a helix bump that changes server names, run
`sync-grammars.py` first, then `sync-lsp-sources.py`.

`sync-lsp-sources.py --requirements-only` rewrites `requirements/` from the checked-in
`sources/` without the network, and adding `--check` exits non-zero when any file in it is
stale, missing or extra.

Note: `sync-lsp-sources.py` is slow by design — it downloads every asset per
server×platform to record checksums. `sync-grammars.py` is a single HTTP fetch.

## sha256 cache and re-pushed tags

`sync-lsp-sources.py` caches sha256 hashes from the previously checked-in
`sources/`, keyed by the full download URL (repo, version, and asset file —
`https://github.com/<repo>/releases/download/<version>/<asset-file>` for github servers, the
recorded url for generic ones), so a routine
re-sync doesn't re-download unchanged assets. This means a version bump always
re-hashes (safe), but
if an upstream maintainer re-pushes a release tag with different bytes under the same
version, the cache serves the old hash and the sync won't notice — exactly the threat
the sha256 pin exists to catch (see `docs/LSP-INSTALL.md`'s "Integrity" note). Run
`python3 scripts/sync-lsp-sources.py --no-cache` periodically (not just on a version
bump) to re-hash everything and catch a re-pushed tag.
