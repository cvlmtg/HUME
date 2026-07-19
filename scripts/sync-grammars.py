#!/usr/bin/env python3
"""Regenerate runtime/scheme/{languages,grammar-sources,lsp-servers}.scm from
helix-editor/helix languages.toml.

Reads the pinned SHA from runtime/scheme/helix-pin.scm, fetches helix's
languages.toml at that commit, and rewrites:
  - languages.scm       — (define-language! …) for every [[language]] block
  - grammar-sources.scm — tree-sitter grammar source catalog
  - lsp-servers.scm     — LSP server registration catalog (see
    docs/LSP-INSTALL.md), derived from [[language]].language-servers and
    [language-server.*]

Idempotent: running twice produces byte-identical files.
"""

import json
import sys
from collections.abc import Callable
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from sync_common import (  # noqa: E402
    fetch_bytes,
    read_pin,
    read_sexpr,
    scheme_list,
    scheme_str,
    write_generated_file,
)

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ImportError:
        sys.exit("error: requires Python 3.11+ or 'pip install tomli'")


REPO = Path(__file__).resolve().parent.parent
HELIX_PIN_SCM = REPO / "runtime" / "scheme" / "helix-pin.scm"
LANGUAGES_SCM = REPO / "runtime" / "scheme" / "languages.scm"
GRAMMAR_SOURCES_SCM = REPO / "runtime" / "scheme" / "grammar-sources.scm"
LSP_SERVERS_SCM = REPO / "runtime" / "scheme" / "lsp-servers.scm"

LANGUAGES_HEADER = """\
;;; runtime/scheme/languages.scm — HUME bundled default language identities.
;;;
;;; Evaluated at startup before init.scm.  Override any entry by redefining
;;; it in init.scm — `define-language!` fully replaces the prior registration
;;; for a given name (see init.scm.example for override examples).
;;;
;;; Identity only: extensions, globs, and shebangs.  No tree-sitter grammars
;;; are shipped here — to enable highlighting, call (register-grammar! …) with
;;; paths to a compiled grammar library and highlights query (typically provided
;;; by a plugin).
;;;
;;; Grammar source metadata lives in grammar-sources.scm (loaded by a grammar-
;;; manager plugin, typically core:plum — not by hume at startup).
;;;
;;; Source: helix-editor/helix languages.toml @ {sha}
;;; Full sync: run scripts/sync-grammars.py after updating helix-pin.scm.
"""

GRAMMAR_SOURCES_HEADER = """\
;;; runtime/scheme/grammar-sources.scm — HUME bundled tree-sitter grammar source catalog.
;;;
;;; PURE DATA. One literal sexpr — a list of (name git-url rev symbol subpath)
;;; 5-tuples. All fields are fully canonicalised; no defaults are applied at
;;; read time. Read via the R7RS idiom from any plugin:
;;;
;;;   (define *grammar-sources*
;;;     (call-with-input-file
;;;       (path-join (runtime-dir) "scheme" "grammar-sources.scm")
;;;       read))
;;;
;;; Source: helix-editor/helix languages.toml @ {sha}
;;; Full sync: run scripts/sync-grammars.py after updating helix-pin.scm.
"""

LSP_SERVERS_HEADER = """\
;;; runtime/scheme/lsp-servers.scm — HUME bundled LSP server registration catalog.
;;;
;;; PURE DATA. One literal sexpr — a list of per-server tagged alists:
;;;
;;;   (name
;;;    (languages (lang-name root-marker…)…)
;;;    (command . cmd)
;;;    (args arg…)
;;;    (config . json-string))
;;;
;;; `args` is the empty tail `(args)` (never #f) when the server takes none.
;;; `config` is Helix's `[language-server.*.config]` table for this server,
;;; copied verbatim — the *entire* tail is `(config)` (never a dotted pair)
;;; when Helix has no config for it; otherwise `(config . "...")`, a single
;;; canonical (sort_keys) JSON-encoded string. `core:lsp/registration.scm`
;;; delivers it two ways, exactly as Helix does: as `initializationOptions`
;;; (the path that actually configures most servers) and as
;;; `register-lsp-server!`'s `#:settings` (answers `workspace/configuration`
;;; pulls — a miss is expected and harmless for servers whose config isn't
;;; nested under their own name). The runtime catalog loader parses the
;;; JSON string with the `(json-parse)` builtin at load time, once, rather
;;; than every plugin needing its own nested-alist/vector-array reader for a
;;; JSON object embedded as Scheme data. All fields are fully canonicalised;
;;; no defaults are applied at read time. Read via the R7RS idiom from any
;;; plugin:
;;;
;;;   (define *lsp-servers*
;;;     (call-with-input-file
;;;       (path-join (runtime-dir) "scheme" "lsp-servers.scm")
;;;       read))
;;;
;;; One server per language — Helix's first-listed ("primary") language-server
;;; only; see docs/LSP-INSTALL.md "v1 scope" for the multi-server rationale.
;;; Install sources (download/build info) live in lsp-sources.scm, joined by
;;; server name.
;;;
;;; Source: helix-editor/helix languages.toml @ {sha}
;;; Full sync: run scripts/sync-grammars.py after updating helix-pin.scm.
"""


def fetch_toml(sha: str) -> dict:
    url = f"https://raw.githubusercontent.com/helix-editor/helix/{sha}/languages.toml"
    return tomllib.loads(fetch_bytes(url, timeout=30).decode())


def parse_grammars(doc: dict) -> dict[str, dict]:
    """Return {name: {url, rev, subpath}} for grammars with a git source."""
    grammars: dict[str, dict] = {}
    for entry in doc.get("grammar", []):
        try:
            name = entry["name"]
            src = entry.get("source", {})
        except KeyError as e:
            sys.exit(f"error: malformed grammar entry in languages.toml: missing key {e}")
        if not src.get("git"):
            print(f"  skip grammar '{name}': no source.git", file=sys.stderr)
            continue
        try:
            grammars[name] = {
                "url": src["git"],
                "rev": src["rev"],
                "subpath": src.get("subpath", ""),
            }
        except KeyError as e:
            sys.exit(f"error: malformed grammar '{name}' source in languages.toml: missing key {e}")
    return grammars


def parse_languages(doc: dict, grammars: dict[str, dict]) -> list[dict]:
    """Return [{name, extensions, globs, shebangs, grammar_name}] for each language."""
    langs = []
    overrides = []
    no_grammar = []

    for entry in doc.get("language", []):
        try:
            name = entry["name"]
        except KeyError as e:
            sys.exit(f"error: malformed language entry in languages.toml: missing key {e}")
        extensions = []
        globs = []
        for ft in entry.get("file-types", []):
            if isinstance(ft, str):
                extensions.append(ft)
            elif isinstance(ft, dict):
                if "glob" in ft:
                    globs.append(ft["glob"])
                else:
                    keys = list(ft.keys())
                    print(
                        f"  skip file-type entry in '{name}': unknown keys {keys}",
                        file=sys.stderr,
                    )
        shebangs = entry.get("shebangs", [])
        grammar_name = entry.get("grammar", name)
        if grammar_name != name:
            overrides.append(f"{name} -> {grammar_name}")
        if grammar_name not in grammars:
            no_grammar.append(name)
        langs.append(
            {
                "name": name,
                "extensions": extensions,
                "globs": globs,
                "shebangs": shebangs,
                "grammar_name": grammar_name,
            }
        )

    if overrides:
        print(f"grammar overrides ({len(overrides)}):", file=sys.stderr)
        for o in overrides:
            print(f"  {o}", file=sys.stderr)
    if no_grammar:
        print(f"languages without a helix grammar ({len(no_grammar)}):", file=sys.stderr)
        for n in no_grammar:
            print(f"  {n}", file=sys.stderr)

    return langs


def emit_grammar_sources(grammars: dict[str, dict], langs: list[dict]) -> list[str]:
    # Start with all direct grammars, then add one alias entry per language that
    # delegates to a different grammar (e.g. jsx → javascript).  Each alias gets
    # the target grammar's URL/rev/subpath but the *target* grammar's symbol,
    # so the compiled .so exports the right C function.
    entries: dict[str, dict] = {}
    for gname, g in grammars.items():
        entries[gname] = {**g, "sym": "tree_sitter_" + gname.replace("-", "_")}

    for lang in langs:
        lname = lang["name"]
        gname = lang["grammar_name"]
        if gname != lname and gname in grammars:
            if lname in entries:
                print(
                    f"  grammar delegation: language '{lname}' delegates to grammar "
                    f"'{gname}', overriding its own same-named direct grammar entry",
                    file=sys.stderr,
                )
            g = grammars[gname]
            entries[lname] = {**g, "sym": "tree_sitter_" + gname.replace("-", "_")}

    subpath_entries = [(n, e["subpath"]) for n, e in entries.items() if e["subpath"]]
    if subpath_entries:
        print(f"subpath grammars ({len(subpath_entries)}):", file=sys.stderr)
        for n, sp in sorted(subpath_entries):
            print(f"  {n}: {sp}", file=sys.stderr)

    # Emit a single literal sexpr (a list of 5-tuples) with no define/provide.
    # Consumers read this via (call-with-input-file path read).
    rows = []
    for name in sorted(entries):
        e = entries[name]
        row = " ({} {} {} {} {})".format(
            scheme_str(name),
            scheme_str(e["url"]),
            scheme_str(e["rev"]),
            scheme_str(e["sym"]),
            scheme_str(e["subpath"]),
        )
        rows.append(row)
    return ["("] + rows + [")"]


def emit_language_identities(langs: list[dict]) -> list[str]:
    lines = []
    for lang in sorted(langs, key=lambda x: x["name"]):
        name_s = scheme_str(lang["name"])
        exts = lang["extensions"]
        globs = lang["globs"]
        shebangs = lang["shebangs"]

        if shebangs:
            lines.append(
                f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)} {scheme_list(shebangs)})"
            )
        elif globs:
            lines.append(f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)})")
        elif exts:
            lines.append(f"(define-language! {name_s} {scheme_list(exts)})")
        else:
            lines.append(f"(define-language! {name_s})")
    return lines


# HUME-specific corrections to upstream Helix `config` data, applied before
# emission. Not Helix bugs to route around blindly — verified against each
# named server's own source that its *real* config-reading code expects a
# different shape than what ships in languages.toml. Add an entry here only
# after checking the server's actual handler, the same way the hostInfo
# rewrite in parse_language_servers below was checked.
CONFIG_OVERRIDES: dict[str, Callable[[dict], dict]] = {
    # Helix's own entry double-wraps this under the server's own name
    # (`[language-server.actions-language-server.config.actions-language-server]`)
    # but connection.ts reads `initializationOptions.sessionToken` flat, no
    # wrapper — the token is silently never seen as shipped upstream.
    "actions-language-server": lambda config: config.get("actions-language-server", config),
    # pony-lsp ignores initializationOptions for these keys entirely; it
    # only reads them from a workspace/configuration pull for section
    # "pony-lsp" (server_options.pony's send_configuration_request), which
    # needs this same data nested one level under that key.
    "pony-lsp": lambda config: {"pony-lsp": config},
}


def parse_language_servers(doc: dict) -> dict[str, dict]:
    """Return {server_name: {command, args, config, languages}}.

    Only the primary (first-listed) language-server per language is kept —
    HUME's client is single-server-per-buffer in v1, so Helix's non-primary
    servers for a language (e.g. python's ["ty", "ruff", "jedi", "pylsp"])
    are not seeded. This also guarantees no two servers ever claim the same
    language, checked by check_lsp_invariants below.

    A language whose primary server has no [language-server.*] table at all
    (upstream gap — e.g. haxe -> haxe-language-server at helix-pin
    8c41b1160792) is skipped with a report, not fatal: the language simply
    has no seedable server, same as a language with no grammar.
    """
    server_defs = doc.get("language-server", {})
    servers: dict[str, dict] = {}
    ignored_keys: set[str] = set()
    broken_servers: dict[str, list[str]] = {}

    for entry in doc.get("language", []):
        lang_name = entry["name"]
        ls_list = entry.get("language-servers", [])
        if not ls_list:
            continue
        primary = ls_list[0]
        # Some entries are inline tables ({ name = "...", except-features = [...] },
        # e.g. gjs/gts/hare) — except-features filters are meaningless to a
        # single-server client, so only the name is kept.
        server_name = primary["name"] if isinstance(primary, dict) else primary
        roots = entry.get("roots", [])

        if server_name in broken_servers:
            broken_servers[server_name].append(lang_name)
            continue

        if server_name not in servers:
            ls_def = server_defs.get(server_name)
            if ls_def is None or "command" not in ls_def:
                broken_servers[server_name] = [lang_name]
                continue
            config = ls_def.get("config")
            if config and "hostInfo" in config:
                config = {**config, "hostInfo": "hume"}
            if config and server_name in CONFIG_OVERRIDES:
                config = CONFIG_OVERRIDES[server_name](config)
            ignored_keys.update(k for k in ls_def if k not in ("command", "args", "config"))
            servers[server_name] = {
                "command": ls_def["command"],
                "args": ls_def.get("args", []),
                "config": config,
                "languages": [],
            }
        servers[server_name]["languages"].append((lang_name, *roots))

    if ignored_keys:
        print(
            f"ignored language-server keys ({len(ignored_keys)}): {sorted(ignored_keys)}",
            file=sys.stderr,
        )
    if broken_servers:
        total = sum(len(v) for v in broken_servers.values())
        print(
            f"skipped {total} language(s) — primary language-server has no "
            f"[language-server.*] command table ({len(broken_servers)} server(s)):",
            file=sys.stderr,
        )
        for server_name, lang_names in sorted(broken_servers.items()):
            print(f"  {server_name}: {', '.join(sorted(lang_names))}", file=sys.stderr)

    return servers


def check_lsp_invariants(servers: dict[str, dict], langs: list[dict]) -> None:
    """Fatal cross-checks: every emitted language exists and belongs to exactly
    one server. Both hold by construction (parse_language_servers only ever
    appends a language to the first server it names) — asserted here so a
    future refactor cannot silently break the invariant."""
    lang_names = {lang["name"] for lang in langs}
    owner: dict[str, str] = {}
    for server_name, s in servers.items():
        for lang_tuple in s["languages"]:
            lang_name = lang_tuple[0]
            if lang_name not in lang_names:
                sys.exit(
                    f"error: lsp-servers.scm invariant violated: language "
                    f"'{lang_name}' (server '{server_name}') is not a known language"
                )
            if lang_name in owner and owner[lang_name] != server_name:
                sys.exit(
                    f"error: lsp-servers.scm invariant violated: language "
                    f"'{lang_name}' claimed by both '{owner[lang_name]}' and '{server_name}'"
                )
            owner[lang_name] = server_name


def emit_lsp_servers(servers: dict[str, dict]) -> list[str]:
    rows = []
    for name in sorted(servers):
        s = servers[name]
        langs_sexpr = " ".join(
            "({})".format(" ".join(scheme_str(x) for x in lang_tuple))
            for lang_tuple in s["languages"]
        )
        args_sexpr = " ".join(scheme_str(a) for a in s["args"])
        config_field = (
            " (config . {})".format(scheme_str(json.dumps(s["config"], sort_keys=True)))
            if s["config"]
            else " (config)"
        )
        row = " ({} (languages {}) (command . {}) (args{}){})".format(
            scheme_str(name),
            langs_sexpr,
            scheme_str(s["command"]),
            f" {args_sexpr}" if args_sexpr else "",
            config_field,
        )
        rows.append(row)
    return ["("] + rows + [")"]


def main() -> None:
    sha = read_pin(HELIX_PIN_SCM)
    print(f"helix-pin: {sha}", file=sys.stderr)

    doc = fetch_toml(sha)
    grammars = parse_grammars(doc)
    langs = parse_languages(doc, grammars)
    servers = parse_language_servers(doc)
    check_lsp_invariants(servers, langs)

    print(
        f"parsed: {len(langs)} languages, {len(grammars)} direct grammars with git source, "
        f"{len(servers)} language servers",
        file=sys.stderr,
    )

    # languages.scm — identity-only. Not a single-literal-sexpr file (a
    # sequence of top-level (define-language! …) forms), so unlike the two
    # below it has no read_sexpr self-check.
    write_generated_file(
        LANGUAGES_SCM, LANGUAGES_HEADER.format(sha=sha), emit_language_identities(langs)
    )

    # grammar-sources.scm — source catalog only
    write_generated_file(
        GRAMMAR_SOURCES_SCM,
        GRAMMAR_SOURCES_HEADER.format(sha=sha),
        emit_grammar_sources(grammars, langs),
    )
    read_sexpr(GRAMMAR_SOURCES_SCM)  # self-check: emitted file must re-parse

    # lsp-servers.scm — LSP server registration catalog
    write_generated_file(
        LSP_SERVERS_SCM, LSP_SERVERS_HEADER.format(sha=sha), emit_lsp_servers(servers)
    )
    read_sexpr(LSP_SERVERS_SCM)  # self-check: emitted file must re-parse


if __name__ == "__main__":
    main()
