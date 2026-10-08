#!/usr/bin/env python3
"""Regenerate runtime/scheme/{languages,grammar-sources}.scm and the lsp-install
catalog files from helix-editor/helix languages.toml.

Reads the pinned SHA from runtime/scheme/helix-pin.scm, fetches helix's
languages.toml at that commit, and rewrites:
  - languages.scm       — (define-language! …) for every [[language]] block,
    with its root markers as `#:roots`
  - grammar-sources.scm — tree-sitter grammar source catalog
  - lsp-install/servers.scm — each server's args and config, derived from
    [language-server.*]
  - lsp-install/language-servers.scm — each language's ordered server list, with
    Helix's only-features/except-features per entry
  - lsp-install/server-commands.scm — the command of each server whose command
    differs from its name, read by the install pipeline and by sync-lsp-sources.py

Idempotent: running twice produces byte-identical files.
"""

from __future__ import annotations

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
LSP_SERVERS_SCM = REPO / "runtime" / "plugins" / "core" / "lsp-install" / "data" / "servers.scm"
LANGUAGE_SERVERS_SCM = (
    REPO / "runtime" / "plugins" / "core" / "lsp-install" / "data" / "language-servers.scm"
)
SERVER_COMMANDS_SCM = (
    REPO / "runtime" / "plugins" / "core" / "lsp-install" / "data" / "server-commands.scm"
)

LANGUAGES_HEADER = """\
;;; runtime/scheme/languages.scm — HUME bundled default language identities.
;;; Generated — do not hand-edit. Record format and load order: README.md, this directory.
;;; Source: helix-editor/helix languages.toml @ {sha}
"""

GRAMMAR_SOURCES_HEADER = """\
;;; runtime/scheme/grammar-sources.scm — HUME bundled tree-sitter grammar source catalog.
;;; Generated — do not hand-edit. Record format and load order: README.md, this directory.
;;; Source: helix-editor/helix languages.toml @ {sha}
"""

LANGUAGE_SERVERS_HEADER = """\
;;; runtime/plugins/core/lsp-install/data/language-servers.scm — each language's ordered LSP server list.
;;; Generated — do not hand-edit. Record format: README.md, this directory.
;;; Source: helix-editor/helix languages.toml @ {sha}
"""

# Helix's `language-servers` feature vocabulary (its LanguageServerFeature
# enum), which HUME's own LspFeature::ALL mirrors; a drift test in
# hume-editor compares the two.
LSP_FEATURES = (
    "format",
    "goto-declaration",
    "goto-definition",
    "goto-type-definition",
    "goto-reference",
    "goto-implementation",
    "signature-help",
    "hover",
    "document-highlight",
    "completion",
    "code-action",
    "document-links",
    "workspace-command",
    "document-symbols",
    "workspace-symbols",
    "diagnostics",
    "pull-diagnostics",
    "rename-symbol",
    "inlay-hints",
    "document-colors",
    "call-hierarchy",
)

LSP_SERVERS_HEADER = """\
;;; runtime/plugins/core/lsp-install/data/servers.scm — HUME bundled LSP server registration catalog.
;;; Generated — do not hand-edit. Record format: README.md, this directory.
;;; Source: helix-editor/helix languages.toml @ {sha}
"""

SERVER_COMMANDS_HEADER = """\
;;; runtime/plugins/core/lsp-install/data/server-commands.scm — the command of each bundled LSP server whose command differs from its name.
;;; Generated — do not hand-edit. Record format: README.md, this directory.
;;; Source: helix-editor/helix languages.toml @ {sha}
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
    """Return [{name, extensions, globs, shebangs, roots, grammar_name, language_id}] for each language."""
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
                "roots": entry.get("roots", []),
                "grammar_name": grammar_name,
                "language_id": entry.get("language-id"),
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
        language_id = lang.get("language_id")
        roots = lang["roots"]
        suffix = f" #:language-id {scheme_str(language_id)}" if language_id else ""
        if roots:
            suffix += f" #:roots {scheme_list(roots)}"

        if shebangs:
            lines.append(
                f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)} "
                f"{scheme_list(shebangs)}{suffix})"
            )
        elif globs:
            lines.append(
                f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)}"
                f"{suffix})"
            )
        elif exts:
            lines.append(f"(define-language! {name_s} {scheme_list(exts)}{suffix})")
        else:
            lines.append(f"(define-language! {name_s}{suffix})")
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


def server_list_entry(element, lang_name: str) -> tuple[str, tuple[str, list[str]] | None]:
    """One [[language]].language-servers element as (name, filter).

    An element is a bare name, or an inline table (`{ name = "...",
    only-features = [...] }`, e.g. gjs/gts/hare) carrying one feature filter:
    `None`, ("only", features) or ("except", features). A table giving both
    filters is fatal, matching what HUME's own `set-language-servers!` rejects.
    """
    if isinstance(element, str):
        return element, None
    name = element["name"]
    only = element.get("only-features")
    except_ = element.get("except-features")
    if only is not None and except_ is not None:
        sys.exit(
            f"error: language '{lang_name}': server '{name}' gives both "
            f"only-features and except-features"
        )
    if only is not None:
        return name, ("only", list(only))
    if except_ is not None:
        return name, ("except", list(except_))
    return name, None


def parse_language_servers(
    doc: dict,
) -> tuple[dict[str, dict], dict[str, list[tuple[str, tuple[str, list[str]] | None]]]]:
    """Return (servers, language_lists).

    `servers` is {server_name: {command, args, config}} for every server any
    language lists. `language_lists` is {language: [(server_name, filter)]}:
    the language's servers in Helix's order (their order is priority order).

    A server with no [language-server.*] command table at all (upstream gap
    — e.g. haxe -> haxe-language-server at helix-pin 8c41b1160792) is left
    out of both, with a report, not fatal: the language simply has no
    seedable entry for it, same as a language with no grammar.
    """
    server_defs = doc.get("language-server", {})
    servers: dict[str, dict] = {}
    language_lists: dict[str, list[tuple[str, tuple[str, list[str]] | None]]] = {}
    ignored_keys: set[str] = set()
    broken_servers: dict[str, list[str]] = {}

    for entry in doc.get("language", []):
        lang_name = entry["name"]

        for element in entry.get("language-servers", []):
            server_name, feature_filter = server_list_entry(element, lang_name)

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
                }
            language_lists.setdefault(lang_name, []).append((server_name, feature_filter))

    if ignored_keys:
        print(
            f"ignored language-server keys ({len(ignored_keys)}): {sorted(ignored_keys)}",
            file=sys.stderr,
        )
    if broken_servers:
        total = sum(len(v) for v in broken_servers.values())
        print(
            f"skipped {total} language(s) — a listed language-server has no "
            f"[language-server.*] command table ({len(broken_servers)} server(s)):",
            file=sys.stderr,
        )
        for server_name, lang_names in sorted(broken_servers.items()):
            print(f"  {server_name}: {', '.join(sorted(lang_names))}", file=sys.stderr)

    return servers, language_lists


def check_lsp_invariants(
    servers: dict[str, dict],
    langs: list[dict],
    language_lists: dict[str, list[tuple[str, tuple[str, list[str]] | None]]],
) -> None:
    """Fatal cross-checks on what the emitted files will say: every language
    exists, every listed server is seeded, no list names a server twice, and
    every feature name is one HUME routes by."""
    lang_names = {lang["name"] for lang in langs}
    for lang_name, entries in language_lists.items():
        if lang_name not in lang_names:
            sys.exit(f"error: language-servers.scm: '{lang_name}' is not a known language")
        seen: set[str] = set()
        for server_name, feature_filter in entries:
            if server_name not in servers:
                sys.exit(
                    f"error: language-servers.scm: '{lang_name}' lists "
                    f"'{server_name}', which servers.scm does not seed"
                )
            if server_name in seen:
                sys.exit(
                    f"error: language-servers.scm: '{lang_name}' lists "
                    f"'{server_name}' twice"
                )
            seen.add(server_name)
            for feature in feature_filter[1] if feature_filter else []:
                if feature not in LSP_FEATURES:
                    sys.exit(
                        f"error: language-servers.scm: '{lang_name}' / '{server_name}': "
                        f"unknown feature '{feature}'"
                    )


def emit_lsp_servers(servers: dict[str, dict]) -> list[str]:
    rows = []
    for name in sorted(servers):
        s = servers[name]
        args_sexpr = " ".join(scheme_str(a) for a in s["args"])
        config_field = (
            " (config . {})".format(scheme_str(json.dumps(s["config"], sort_keys=True)))
            if s["config"]
            else " (config)"
        )
        row = " ({} (args{}){})".format(
            scheme_str(name),
            f" {args_sexpr}" if args_sexpr else "",
            config_field,
        )
        rows.append(row)
    return ["("] + rows + [")"]


def emit_server_commands(servers: dict[str, dict]) -> list[str]:
    rows = [
        f" ({scheme_str(name)} . {scheme_str(servers[name]['command'])})"
        for name in sorted(servers)
        if servers[name]["command"] != name
    ]
    return ["("] + rows + [")"]


def emit_language_servers(
    language_lists: dict[str, list[tuple[str, tuple[str, list[str]] | None]]],
) -> list[str]:
    rows = []
    for lang_name in sorted(language_lists):
        entries = []
        for server_name, feature_filter in language_lists[lang_name]:
            parts = [scheme_str(server_name)]
            if feature_filter:
                kind, features = feature_filter
                parts.append(
                    "({}-features {})".format(kind, " ".join(scheme_str(f) for f in features))
                )
            entries.append("({})".format(" ".join(parts)))
        rows.append(" ({} (servers {}))".format(scheme_str(lang_name), " ".join(entries)))
    return ["("] + rows + [")"]


def main() -> None:
    sha = read_pin(HELIX_PIN_SCM)
    print(f"helix-pin: {sha}", file=sys.stderr)

    doc = fetch_toml(sha)
    grammars = parse_grammars(doc)
    langs = parse_languages(doc, grammars)
    servers, language_lists = parse_language_servers(doc)
    check_lsp_invariants(servers, langs, language_lists)

    print(
        f"parsed: {len(langs)} languages, {len(grammars)} direct grammars with git source, "
        f"{len(servers)} language servers",
        file=sys.stderr,
    )

    # languages.scm — identity and root markers. Not a single-literal-sexpr file (a
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

    # servers.scm — LSP server args and config
    write_generated_file(
        LSP_SERVERS_SCM, LSP_SERVERS_HEADER.format(sha=sha), emit_lsp_servers(servers)
    )
    read_sexpr(LSP_SERVERS_SCM)  # self-check: emitted file must re-parse

    # server-commands.scm — each server's command
    write_generated_file(
        SERVER_COMMANDS_SCM,
        SERVER_COMMANDS_HEADER.format(sha=sha),
        emit_server_commands(servers),
    )
    read_sexpr(SERVER_COMMANDS_SCM)  # self-check: emitted file must re-parse

    # language-servers.scm — each language's ordered server list
    write_generated_file(
        LANGUAGE_SERVERS_SCM,
        LANGUAGE_SERVERS_HEADER.format(sha=sha),
        emit_language_servers(language_lists),
    )
    read_sexpr(LANGUAGE_SERVERS_SCM)  # self-check: emitted file must re-parse


if __name__ == "__main__":
    main()
