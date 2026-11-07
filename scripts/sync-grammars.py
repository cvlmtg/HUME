#!/usr/bin/env python3
"""Regenerate runtime/scheme/languages.scm, runtime/scheme/languages/*.scm and the
lsp-install catalog files from cvlmtg/treesitter.hume's language settings.

Reads the treesitter.hume release tag core:plum pins
(runtime/plugins/core/plum/data/treesitter-hume-pin.scm), so the language data
and the grammars come from one release, fetches that tag's source, reads its
languages/<name>/language.toml files and language-servers.toml (Helix's
languages.toml settings, with treesitter.hume's corrections), and rewrites:
  - languages.scm       — (define-language! …) detection data for every [[language]] block
  - languages/<name>.scm — one data file per [[language]] block: comment tokens,
    language-id and root markers
  - lsp-install/data/registrations/<server>.scm — one file per server: its command,
    args and config, each omitted when it is the default (the server's name, no
    args, no config), derived from [language-server.*]
  - lsp-install/data/language-servers.scm — each language's ordered server list, with
    Helix's only-features/except-features per entry

Idempotent: running twice produces byte-identical files.
"""

from __future__ import annotations

import io
import json
import re
import sys
import tarfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from sync_common import (  # noqa: E402
    check_file_names,
    fetch_bytes,
    read_pin,
    read_sexpr,
    scheme_list,
    scheme_str,
    scheme_str_list,
    write_generated_file,
    write_named_files,
)

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ImportError:
        sys.exit("error: requires Python 3.11+ or 'pip install tomli'")


REPO = Path(__file__).resolve().parent.parent
TREESITTER_PIN_SCM = REPO / "runtime" / "plugins" / "core" / "plum" / "data" / "treesitter-hume-pin.scm"
LANGUAGES_SCM = REPO / "runtime" / "scheme" / "languages.scm"
LANGUAGES_DIR = REPO / "runtime" / "scheme" / "languages"
LSP_INSTALL_DATA = REPO / "runtime" / "plugins" / "core" / "lsp-install" / "data"
LSP_REGISTRATIONS_DIR = LSP_INSTALL_DATA / "registrations"
LANGUAGE_SERVERS_SCM = LSP_INSTALL_DATA / "language-servers.scm"

LANGUAGES_HEADER = """\
;;; runtime/scheme/languages.scm — HUME bundled default language identities.
;;; Generated — do not hand-edit. Record format and load order: README.md, this directory.
;;; Source: cvlmtg/treesitter.hume {tag}
"""

LANGUAGE_DATA_HEADER = """\
;;; runtime/scheme/languages/{name}.scm — HUME bundled language data.
;;; Generated — do not hand-edit. Record format: README.md, parent directory.
;;; Source: cvlmtg/treesitter.hume {tag}
"""

LANGUAGE_SERVERS_HEADER = """\
;;; runtime/plugins/core/lsp-install/data/language-servers.scm — each language's ordered LSP server list.
;;; Generated — do not hand-edit. Record format: docs/catalogs.md, plugin directory.
;;; Source: cvlmtg/treesitter.hume {tag}
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

LSP_REGISTRATION_HEADER = """\
;;; runtime/plugins/core/lsp-install/data/registrations/{name}.scm — one LSP server's registration.
;;; Generated — do not hand-edit. Record format: docs/catalogs.md, plugin directory.
;;; Source: cvlmtg/treesitter.hume {tag}
"""


LANGUAGE_FILE = re.compile(r"[^/]+/languages/([^/]+)/language\.toml")
SERVERS_FILE = re.compile(r"[^/]+/language-servers\.toml")


def language_document(archive: bytes) -> dict:
    """The `{language: [...], language-server: {...}}` document a
    treesitter.hume source archive's language settings make, its languages
    in name order. Each language.toml holds the one `[[language]]` its
    folder names."""
    languages = []
    servers = None
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
        for member in tar:
            if not member.isfile():
                continue
            language = LANGUAGE_FILE.fullmatch(member.name)
            if not language and not SERVERS_FILE.fullmatch(member.name):
                continue
            file = tar.extractfile(member)
            if file is None:
                sys.exit(f"error: {member.name}: unreadable")
            doc = tomllib.loads(file.read().decode())
            if language is None:
                servers = doc.get("language-server")
                continue
            entries = doc.get("language", [])
            if [entry.get("name") for entry in entries] != [language[1]]:
                sys.exit(f"error: {member.name} must hold one [[language]] named '{language[1]}'")
            languages.append(entries[0])
    if servers is None:
        sys.exit("error: the archive has no language-servers.toml")
    if not languages:
        sys.exit("error: the archive has no languages/<name>/language.toml")
    return {
        "language": sorted(languages, key=lambda entry: entry["name"]),
        "language-server": servers,
    }


def fetch_language_document(tag: str) -> dict:
    url = f"https://github.com/cvlmtg/treesitter.hume/archive/refs/tags/{tag}.tar.gz"
    return language_document(fetch_bytes(url, timeout=60))


def comment_tokens(entry: dict) -> list[str]:
    """`comment-token`/`comment-tokens` as a list, accepting the shapes Helix's
    `from_comment_tokens` accepts: one string, or a list of strings."""
    name = entry["name"]
    value = entry.get("comment-tokens", entry.get("comment-token"))
    if value is None:
        return []
    if isinstance(value, str):
        return [value]
    if isinstance(value, list) and all(isinstance(t, str) for t in value):
        return list(value)
    sys.exit(f"error: language '{name}': malformed comment-tokens {value!r}")


def block_comment_tokens(entry: dict) -> list[tuple[str, str]]:
    """`block-comment-tokens` as (start, end) pairs, accepting the shapes Helix's
    `from_block_comment_tokens` accepts: one `{start, end}` table, a list of
    them, or a positional `[start, end]` pair."""
    name = entry["name"]
    value = entry.get("block-comment-tokens")

    def pair(table) -> tuple[str, str]:
        if (
            isinstance(table, dict)
            and set(table) == {"start", "end"}
            and all(isinstance(v, str) for v in table.values())
        ):
            return table["start"], table["end"]
        sys.exit(f"error: language '{name}': malformed block-comment-tokens {value!r}")

    if value is None:
        return []
    if isinstance(value, dict):
        return [pair(value)]
    if isinstance(value, list) and len(value) == 2 and all(isinstance(t, str) for t in value):
        return [(value[0], value[1])]
    if isinstance(value, list):
        return [pair(t) for t in value]
    sys.exit(f"error: language '{name}': malformed block-comment-tokens {value!r}")


def checked_shebangs(entry: dict) -> list[str]:
    """`entry`'s shebangs. Fatal when one is not one interpreter name, which
    `define-language!` refuses (`ShebangInterpreter::new`)."""
    name = entry["name"]
    shebangs = entry.get("shebangs", [])
    for shebang in shebangs:
        if not shebang or any(c.isspace() or c in "/\\" for c in shebang):
            sys.exit(
                f"error: language '{name}': shebang {shebang!r} is not one interpreter "
                f"name; correct it in treesitter.hume's languages/{name}/language.toml"
            )
    return list(shebangs)


def parse_languages(doc: dict) -> list[dict]:
    """Return one dict per language: detection data (name, extensions, globs,
    shebangs) and per-language data (comment_tokens, block_comment_tokens,
    language_id, roots)."""
    langs = []

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
        shebangs = checked_shebangs(entry)
        langs.append(
            {
                "name": name,
                "extensions": extensions,
                "globs": globs,
                "shebangs": shebangs,
                "comment_tokens": comment_tokens(entry),
                "block_comment_tokens": block_comment_tokens(entry),
                "language_id": entry.get("language-id"),
                "roots": entry.get("roots", []),
            }
        )

    return langs


def emit_language_identities(langs: list[dict]) -> list[str]:
    lines = []
    for lang in sorted(langs, key=lambda x: x["name"]):
        name_s = scheme_str(lang["name"])
        exts = lang["extensions"]
        globs = lang["globs"]
        shebangs = lang["shebangs"]

        if shebangs:
            lines.append(
                f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)} "
                f"{scheme_list(shebangs)})"
            )
        elif globs:
            lines.append(f"(define-language! {name_s} {scheme_list(exts)} {scheme_list(globs)})")
        elif exts:
            lines.append(f"(define-language! {name_s} {scheme_list(exts)})")
        else:
            lines.append(f"(define-language! {name_s})")
    return lines


def emit_language_data(lang: dict) -> list[str]:
    """One language's data file body: a single literal alist, each key present
    only when the language has a value for it."""
    entries = []
    if lang["comment_tokens"]:
        entries.append("(comment-tokens {})".format(scheme_str_list(lang["comment_tokens"])))
    if lang["block_comment_tokens"]:
        pairs = " ".join(
            "({} . {})".format(scheme_str(start), scheme_str(end))
            for start, end in lang["block_comment_tokens"]
        )
        entries.append(f"(block-comment-tokens {pairs})")
    if lang["language_id"] and lang["language_id"] != lang["name"]:
        entries.append("(language-id . {})".format(scheme_str(lang["language_id"])))
    if lang["roots"]:
        entries.append("(roots {})".format(scheme_str_list(lang["roots"])))
    if not entries:
        return ["()"]
    rows = ["(" + entries[0]] + [" " + e for e in entries[1:]]
    rows[-1] += ")"
    return rows


def write_language_files(directory: Path, tag: str, langs: list[dict]) -> None:
    """One data file per language in `langs`, through `write_named_files`."""
    write_named_files(
        directory,
        lambda name: LANGUAGE_DATA_HEADER.format(name=name, tag=tag),
        {lang["name"]: emit_language_data(lang) for lang in langs},
    )


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
    — e.g. haxe -> haxe-language-server at Helix 8c41b1160792) is left
    out of both, with a report, not fatal: the language simply has no
    seedable entry for it.
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
                    f"'{server_name}', which registrations/ does not seed"
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


def emit_lsp_registration(name: str, server: dict) -> list[str]:
    """One server's registration file body: the fields that differ from the
    defaults, or the empty list."""
    fields = []
    if server["command"] != name:
        fields.append("(command . {})".format(scheme_str(server["command"])))
    if server["args"]:
        fields.append("(args {})".format(scheme_str_list(server["args"])))
    if server["config"]:
        fields.append(
            "(config . {})".format(scheme_str(json.dumps(server["config"], sort_keys=True)))
        )
    return ["({})".format(" ".join(fields))]


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
                    "({}-features {})".format(kind, scheme_str_list(features))
                )
            entries.append("({})".format(" ".join(parts)))
        rows.append(" ({} (servers {}))".format(scheme_str(lang_name), " ".join(entries)))
    return ["("] + rows + [")"]


def main() -> None:
    tag = read_pin(TREESITTER_PIN_SCM)

    doc = fetch_language_document(tag)
    langs = parse_languages(doc)
    check_file_names("language", (lang["name"] for lang in langs))
    servers, language_lists = parse_language_servers(doc)
    check_file_names("server", servers)
    check_lsp_invariants(servers, langs, language_lists)

    print(
        f"parsed: {len(langs)} languages, {len(servers)} language servers",
        file=sys.stderr,
    )

    # languages.scm — detection data. Not a single-literal-sexpr file (a
    # sequence of top-level (define-language! …) forms), so unlike the others
    # it has no read_sexpr self-check.
    write_generated_file(
        LANGUAGES_SCM, LANGUAGES_HEADER.format(tag=tag), emit_language_identities(langs)
    )

    # languages/<name>.scm — per-language data
    write_language_files(LANGUAGES_DIR, tag, langs)

    # lsp-install registrations/<server>.scm — command, args and config, defaults omitted
    write_named_files(
        LSP_REGISTRATIONS_DIR,
        lambda name: LSP_REGISTRATION_HEADER.format(name=name, tag=tag),
        {name: emit_lsp_registration(name, servers[name]) for name in sorted(servers)},
    )

    # language-servers.scm — each language's ordered server list
    write_generated_file(
        LANGUAGE_SERVERS_SCM,
        LANGUAGE_SERVERS_HEADER.format(tag=tag),
        emit_language_servers(language_lists),
    )
    read_sexpr(LANGUAGE_SERVERS_SCM)  # self-check: emitted file must re-parse


if __name__ == "__main__":
    main()
