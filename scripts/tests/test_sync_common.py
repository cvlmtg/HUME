"""Tests for the shared writers in scripts/sync_common.py."""

import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))
import sync_common  # noqa: E402


def header(name):
    return f";;; {name}.scm — test header.\n"


class WriteNamedFiles(unittest.TestCase):
    def write(self, directory, bodies):
        with contextlib.redirect_stderr(io.StringIO()):
            sync_common.write_named_files(directory, header, bodies)

    def test_writes_one_file_per_name_with_its_header_and_body(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / "named"

            self.write(directory, {"markdown.inline": ['((a . "x")', " (b 1))"], "c": ["()"]})

            self.assertEqual(sorted(p.name for p in directory.iterdir()), ["c.scm", "markdown.inline.scm"])
            text = (directory / "markdown.inline.scm").read_text(encoding="utf-8")
        self.assertEqual(text, ';;; markdown.inline.scm — test header.\n\n((a . "x")\n (b 1))\n')

    def test_removes_the_file_of_a_name_that_is_gone(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "gone.scm").write_text("()\n", encoding="utf-8")

            self.write(directory, {"kept": ["()"]})

            self.assertEqual(sorted(p.name for p in directory.iterdir()), ["kept.scm"])

    def test_a_dotfile_is_left_alone(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / ".DS_Store").write_bytes(b"finder")

            self.write(directory, {"kept": ["()"]})

            self.assertEqual(sorted(p.name for p in directory.iterdir()), [".DS_Store", "kept.scm"])

    def test_a_file_that_is_not_scm_is_fatal(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "notes.txt").write_text("hand-written\n", encoding="utf-8")

            with self.assertRaises(SystemExit):
                self.write(directory, {"kept": ["()"]})

    def test_a_body_that_does_not_parse_is_fatal(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                self.write(Path(tmp), {"bad": ["((a"]})


class ReadNamedFiles(unittest.TestCase):
    def test_reads_each_body_by_name_sorted_skipping_dotfiles(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            with contextlib.redirect_stderr(io.StringIO()):
                sync_common.write_named_files(
                    directory, header, {"b": ['((kind . npm))'], "a.b": ["()"]}
                )
            (directory / ".DS_Store").write_bytes(b"finder")

            bodies = sync_common.read_named_files(directory)

        self.assertEqual(list(bodies), ["a.b", "b"])
        self.assertEqual(bodies["a.b"], [])
        self.assertEqual(bodies["b"], [("kind", "npm")])

    def test_a_missing_directory_reads_as_no_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(sync_common.read_named_files(Path(tmp) / "missing"), {})


if __name__ == "__main__":
    unittest.main()
