# Sync scripts

Dev-time scripts that regenerate the checked-in seeded data files in `runtime/scheme/`
from pinned upstream revisions. One script per pin; the editor never parses upstream
formats at runtime. Design rationale for the LSP parts: `docs/LSP-INSTALL.md`.

| Script | Pin | Fetches | Emits |
|---|---|---|---|
| `sync-grammars.py` | `runtime/scheme/helix-pin.scm` | `helix-editor/helix` `languages.toml` | `languages.scm`, `grammar-sources.scm`, `lsp-servers.scm` |
| `sync-lsp-sources.py` | `runtime/scheme/mason-pin.scm` | `mason-org/mason-registry` `registry.json.zip` + every release asset (for sha256) | `lsp-sources.scm` |

Shared helpers (pin reading, sexpr emission, atomic writes) live in `sync_common.py`.

## Run order

Each script runs alone after its own pin bump:

- bump `helix-pin.scm` → run `sync-grammars.py`
- bump `mason-pin.scm` → run `sync-lsp-sources.py`

One exception: `sync-lsp-sources.py` reads the checked-in `lsp-servers.scm` to filter
Mason to the servers Helix actually wires — through an explicit Helix→Mason name-mapping
table (the namespaces differ: Helix `pylsp` is Mason `python-lsp-server`), reporting every
Helix server left unmatched. So after a helix bump that changes server names, run
`sync-grammars.py` first, then `sync-lsp-sources.py`.

Note: `sync-lsp-sources.py` is slow by design — it downloads every asset per
server×platform to record checksums. `sync-grammars.py` is a single HTTP fetch.

## sha256 cache and re-pushed tags

`sync-lsp-sources.py` caches sha256 hashes from the previously checked-in
`lsp-sources.scm`, keyed by (version, asset file), so a routine re-sync doesn't
re-download unchanged assets. This means a version bump always re-hashes (safe), but
if an upstream maintainer re-pushes a release tag with different bytes under the same
version, the cache serves the old hash and the sync won't notice — exactly the threat
the sha256 pin exists to catch (see `docs/LSP-INSTALL.md`'s "Integrity" note). Run
`python3 scripts/sync-lsp-sources.py --no-cache` periodically (not just on a version
bump) to re-hash everything and catch a re-pushed tag.
