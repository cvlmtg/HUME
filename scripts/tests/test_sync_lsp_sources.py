"""Tests for the offline requirements step in scripts/sync-lsp-sources.py."""

import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))
import sync_common  # noqa: E402

_spec = importlib.util.spec_from_file_location("sync_lsp_sources", SCRIPTS / "sync-lsp-sources.py")
sync = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sync)

NPM_SOURCE = ['((kind . npm) (version . "1") (packages "x@1") (bin . "x"))']


class WriteRequirementsCheck(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.sources = root / "sources"
        self.out = root / "requirements"
        with contextlib.redirect_stderr(io.StringIO()):
            sync_common.write_named_files(
                self.sources, lambda name: f";;; {name}\n", {"a-ls": NPM_SOURCE, "b-ls": NPM_SOURCE}
            )
            sync.write_requirements(self.sources, self.out, check=False)

    def tearDown(self):
        self.tmp.cleanup()

    def check(self):
        sync.write_requirements(self.sources, self.out, check=True)

    def test_the_written_files_pass(self):
        self.check()

    def test_a_missing_file_fails(self):
        (self.out / "a-ls.scm").unlink()
        with self.assertRaises(SystemExit):
            self.check()

    def test_an_extra_file_fails(self):
        (self.out / "c-ls.scm").write_text("()\n", encoding="utf-8")
        with self.assertRaises(SystemExit):
            self.check()

    def test_a_dotfile_is_ignored(self):
        (self.out / ".DS_Store").write_bytes(b"finder")
        self.check()


if __name__ == "__main__":
    unittest.main()
