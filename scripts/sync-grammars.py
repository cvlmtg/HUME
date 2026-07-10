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

import sys
import urllib.request
from pathlib import Path
from urllib.error import URLError

sys.path.insert(0, str(Path(__file__).resolve().parent))
from sync_common import (  # noqa: E402
    read_pin,
    scheme_list,
    scheme_str,
    sexpr_dumps,
    write_atomic,
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
;;;    (settings (key . value)…))
;;;
;;; Absent/empty fields are the empty tail — (args), (settings) — never #f.
;;; All fields are fully canonicalised; no defaults are applied at read time.
;;; Read via the R7RS idiom from any plugin:
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
    print(f"fetching {url}", file=sys.stderr)
    try:
        with urllib.request.urlopen(url, timeout=30) as r:
            return tomllib.loads(r.read().decode())
    except URLError as e:
        sys.exit(f"error: failed to fetch {url}: {e}")


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
        name = entry["name"]
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
        if gname != lname and gname in grammars and lname not in entries:
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


def parse_language_servers(doc: dict) -> dict[str, dict]:
    """Return {server_name: {command, args, settings, languages}}.

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
            settings = ls_def.get("config")
            if settings and "hostInfo" in settings:
                settings = {**settings, "hostInfo": "hume"}
            ignored_keys.update(k for k in ls_def if k not in ("command", "args", "config"))
            servers[server_name] = {
                "command": ls_def["command"],
                "args": ls_def.get("args", []),
                "settings": settings,
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
        settings_sexpr = sexpr_dumps(s["settings"]) if s["settings"] else ""
        row = " ({} (languages {}) (command . {}) (args{}) (settings{}))".format(
            scheme_str(name),
            langs_sexpr,
            scheme_str(s["command"]),
            f" {args_sexpr}" if args_sexpr else "",
            f" {settings_sexpr}" if settings_sexpr else "",
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

    # languages.scm — identity-only
    langs_parts = [LANGUAGES_HEADER.format(sha=sha)]
    langs_parts.append("\n".join(emit_language_identities(langs)))
    langs_parts.append("")  # trailing newline
    write_atomic(LANGUAGES_SCM, "\n".join(langs_parts))

    # grammar-sources.scm — source catalog only
    src_parts = [GRAMMAR_SOURCES_HEADER.format(sha=sha)]
    src_parts.append("\n".join(emit_grammar_sources(grammars, langs)))
    src_parts.append("")  # trailing newline
    write_atomic(GRAMMAR_SOURCES_SCM, "\n".join(src_parts))

    # lsp-servers.scm — LSP server registration catalog
    lsp_parts = [LSP_SERVERS_HEADER.format(sha=sha)]
    lsp_parts.append("\n".join(emit_lsp_servers(servers)))
    lsp_parts.append("")  # trailing newline
    write_atomic(LSP_SERVERS_SCM, "\n".join(lsp_parts))


if __name__ == "__main__":
    main()
