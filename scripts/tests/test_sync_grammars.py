"""Tests for the LSP server parsing in scripts/sync-grammars.py."""

import contextlib
import importlib.util
import io
import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))
_spec = importlib.util.spec_from_file_location("sync_grammars", SCRIPTS / "sync-grammars.py")
sync = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sync)

# Helix's `language-servers` feature names, written out from helix's
# `LanguageServerFeature` enum, independent of the script's own constant.
HELIX_FEATURES = {
    "format", "goto-declaration", "goto-definition", "goto-type-definition",
    "goto-reference", "goto-implementation", "signature-help", "hover",
    "document-highlight", "completion", "code-action", "document-links",
    "workspace-command", "document-symbols", "workspace-symbols", "diagnostics",
    "pull-diagnostics", "rename-symbol", "inlay-hints", "document-colors",
    "call-hierarchy",
}


def language(name, servers, roots=()):
    return {"name": name, "language-servers": servers, "roots": list(roots)}


def server_table(*names):
    return {name: {"command": name, "args": []} for name in names}


def parse(doc):
    with contextlib.redirect_stderr(io.StringIO()):
        return sync.parse_language_servers(doc)


class ParseLanguageServers(unittest.TestCase):
    def test_every_listed_server_is_kept_in_order(self):
        doc = {
            "language": [language("python", ["ty", "ruff", "pylsp"], ["pyproject.toml"])],
            "language-server": server_table("ty", "ruff", "pylsp"),
        }

        _, lists = parse(doc)

        self.assertEqual(lists["python"], [("ty", None), ("ruff", None), ("pylsp", None)])

    def test_a_language_list_carries_no_roots(self):
        doc = {
            "language": [language("typescript", ["tsls"], ["tsconfig.json"])],
            "language-server": server_table("tsls"),
        }

        servers, lists = parse(doc)

        self.assertEqual(lists["typescript"], [("tsls", None)])
        self.assertEqual(set(servers["tsls"]), {"command", "args", "config"})

    def test_inline_table_filters_are_carried_through(self):
        doc = {
            "language": [
                language(
                    "gjs",
                    [
                        {"name": "tsls", "except-features": ["format", "diagnostics"]},
                        {"name": "ember", "only-features": ["completion"]},
                    ],
                )
            ],
            "language-server": server_table("tsls", "ember"),
        }

        _, lists = parse(doc)

        self.assertEqual(
            lists["gjs"],
            [
                ("tsls", ("except", ["format", "diagnostics"])),
                ("ember", ("only", ["completion"])),
            ],
        )

    def test_both_filters_on_one_entry_are_fatal(self):
        doc = {
            "language": [
                language(
                    "gjs",
                    [{"name": "tsls", "only-features": ["hover"], "except-features": ["format"]}],
                )
            ],
            "language-server": server_table("tsls"),
        }

        with self.assertRaises(SystemExit):
            parse(doc)

    def test_a_server_with_no_command_table_is_dropped_from_the_lists_and_reported(self):
        doc = {
            "language": [language("haxe", ["haxe-language-server", "other"])],
            "language-server": server_table("other"),
        }
        stderr = io.StringIO()

        with contextlib.redirect_stderr(stderr):
            servers, lists = sync.parse_language_servers(doc)

        self.assertEqual(lists["haxe"], [("other", None)])
        self.assertNotIn("haxe-language-server", servers)
        self.assertIn("haxe-language-server", stderr.getvalue())


class ParseLanguages(unittest.TestCase):
    def test_roots_are_carried_per_language(self):
        doc = {
            "language": [
                {"name": "go", "file-types": ["go"], "roots": ["go.work", "go.mod"]},
                {"name": "gomod", "file-types": ["mod"]},
            ]
        }

        with contextlib.redirect_stderr(io.StringIO()):
            langs = sync.parse_languages(doc, {})

        self.assertEqual([lang["roots"] for lang in langs], [["go.work", "go.mod"], []])


class CheckInvariants(unittest.TestCase):
    LANGS = [{"name": "python"}]
    SERVERS = {"ty": {}, "ruff": {}}

    def check(self, servers_by_lang):
        sync.check_lsp_invariants(self.SERVERS, self.LANGS, servers_by_lang)

    def test_accepts_two_servers_for_one_language(self):
        self.check({"python": [("ty", None), ("ruff", ("only", ["format"]))]})

    def test_rejects_an_unknown_feature_name(self):
        with self.assertRaises(SystemExit):
            self.check({"python": [("ty", ("only", ["formatting"]))]})

    def test_rejects_a_duplicate_server_in_one_list(self):
        with self.assertRaises(SystemExit):
            self.check({"python": [("ty", None), ("ty", None)]})

    def test_rejects_a_listed_server_that_is_not_seeded(self):
        with self.assertRaises(SystemExit):
            self.check({"python": [("ty", None), ("ghost", None)]})

    def test_rejects_a_list_for_an_unknown_language(self):
        with self.assertRaises(SystemExit):
            self.check({"cobol": [("ty", None)]})


class Emit(unittest.TestCase):
    def test_language_servers_rows(self):
        rows = sync.emit_language_servers(
            {
                "python": [("ty", None), ("ruff", ("only", ["format", "diagnostics"]))],
                "gjs": [("tsls", ("except", ["format"]))],
            }
        )

        self.assertEqual(
            rows,
            [
                "(",
                ' ("gjs" (servers ("tsls" (except-features "format"))))',
                ' ("python" (servers ("ty") ("ruff" (only-features "format" "diagnostics"))))',
                ")",
            ],
        )

    def test_identity_rows_carry_roots(self):
        base = {"extensions": ["go"], "globs": [], "shebangs": []}
        rows = sync.emit_language_identities(
            [
                {"name": "go", **base, "roots": ["go.work", "go.mod"], "language_id": None},
                {"name": "gomod", **base, "roots": [], "language_id": None},
                {"name": "tsx", **base, "roots": ["package.json"], "language_id": "typescriptreact"},
            ]
        )

        self.assertEqual(
            rows,
            [
                """(define-language! "go" '("go") #:roots '("go.work" "go.mod"))""",
                """(define-language! "gomod" '("go"))""",
                """(define-language! "tsx" '("go") #:language-id "typescriptreact" """
                """#:roots '("package.json"))""",
            ],
        )

    def test_servers_rows_carry_only_args_and_config(self):
        rows = sync.emit_lsp_servers(
            {
                "b": {"command": "bb", "args": ["--stdio"], "config": {"x": 1}},
                "a": {"command": "aa", "args": [], "config": None},
            }
        )

        self.assertEqual(
            rows,
            [
                "(",
                ' ("a" (args) (config))',
                ' ("b" (args "--stdio") (config . "{\\"x\\": 1}"))',
                ")",
            ],
        )

    def test_server_commands_rows_list_only_commands_that_differ_from_the_name(self):
        rows = sync.emit_server_commands(
            {
                "same": {"command": "same", "args": [], "config": None},
                "b": {"command": "bb", "args": [], "config": None},
                "a": {"command": "aa", "args": [], "config": None},
            }
        )

        self.assertEqual(rows, ["(", ' ("a" . "aa")', ' ("b" . "bb")', ")"])

    def test_the_feature_vocabulary_is_helixs(self):
        self.assertEqual(set(sync.LSP_FEATURES), HELIX_FEATURES)
        self.assertEqual(len(sync.LSP_FEATURES), len(HELIX_FEATURES))


if __name__ == "__main__":
    unittest.main()
