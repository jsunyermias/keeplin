"""Mutation tests for the filesystem-format policy gate."""

import importlib.util
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO = Path(os.environ.get("FORMAT_POLICY_REPO", Path(__file__).resolve().parents[2])).resolve()
SPEC = importlib.util.spec_from_file_location(
    "format_policy", REPO / "scripts" / "check-filesystem-format-policy.py"
)
POLICY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(POLICY)


class FilesystemFormatPolicyCheck(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.git("init", "-q")
        self.git("config", "user.email", "fixture@example.com")
        self.git("config", "user.name", "Fixture")
        self.git("config", "commit.gpgsign", "false")
        self.write_lifecycle(8, "")
        self.commit("base")
        self.base = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.root, capture_output=True, text=True, check=True
        ).stdout

    def write(self, path, text):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def write_lifecycle(self, version, dispatch):
        self.write(
            POLICY.LIFECYCLE,
            f"impl FsBackend {{\n    pub(super) const FORMAT_VERSION: u32 = {version};\n{dispatch}}}\n",
        )

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD").strip()

    def evaluate(self, head):
        return POLICY.evaluate(self.root, self.base, head)

    def test_unrelated_change_passes(self):
        self.write("README.md", "unrelated\n")
        self.assertEqual(self.evaluate(self.commit("docs")), [])

    def test_version_bump_without_evidence_fails_and_disclaims_substantive_judgment(self):
        self.write_lifecycle(9, "")
        failures = self.evaluate(self.commit("bump"))
        self.assertEqual(len(failures), 1)
        self.assertIn("v8_to_v9", failures[0])
        self.assertIn("cannot judge", failures[0])

    def test_dispatch_and_named_preservation_test_pass(self):
        dispatch = "    async fn apply_format_migration(&self) { migrate(); }\n"
        self.write_lifecycle(9, dispatch)
        self.write(
            "keeplin-core/src/storage/fs/tests.rs",
            "#[test]\nfn populated_data_preservation_v8_to_v9() {}\n",
        )
        self.assertEqual(self.evaluate(self.commit("real migration")), [])

    def test_unchanged_dispatch_does_not_satisfy_a_bump(self):
        dispatch = "    async fn apply_format_migration(&self) { migrate(); }\n"
        self.write_lifecycle(8, dispatch)
        self.base = self.commit("existing dispatch")
        self.write_lifecycle(9, dispatch)
        self.write(
            "keeplin-core/src/storage/fs/tests.rs",
            "#[test]\nfn populated_data_preservation_v8_to_v9() {}\n",
        )
        self.assertTrue(self.evaluate(self.commit("version only")))

    def test_accepted_bounded_exception_cited_by_commit_passes(self):
        self.write(
            "docs/adr/0017-format-exception.md",
            "# Exception\n\n- Status: accepted\n\nFORMAT_VERSION bounded exception.\n",
        )
        self.base = self.commit("accept exception")
        self.write_lifecycle(9, "")
        self.assertEqual(self.evaluate(self.commit("ADR 0017")), [])

    def test_proposed_exception_does_not_pass(self):
        self.write(
            "docs/adr/0017-format-exception.md",
            "# Exception\n\n- Status: proposed\n\nFORMAT_VERSION bounded exception.\n",
        )
        self.base = self.commit("propose exception")
        self.write_lifecycle(9, "")
        self.assertTrue(self.evaluate(self.commit("ADR 0017")))

    def test_exception_accepted_inside_bump_is_not_separately_accepted(self):
        self.write_lifecycle(9, "")
        self.write(
            "docs/adr/0017-format-exception.md",
            "# Exception\n\n- Status: accepted\n\nFORMAT_VERSION bounded exception.\n",
        )
        self.assertTrue(self.evaluate(self.commit("ADR 0017 with bump")))

    def test_latch_can_be_added_once(self):
        self.write(POLICY.LATCH, '{"tag":"v0.1.0"}\n')
        self.assertEqual(self.evaluate(self.commit("add latch")), [])

    def test_latch_cannot_be_modified_in_a_later_commit(self):
        self.write(POLICY.LATCH, '{"tag":"v0.1.0"}\n')
        self.commit("add latch")
        self.write(POLICY.LATCH, '{"tag":"v0.1.1"}\n')
        failures = self.evaluate(self.commit("modify latch"))
        self.assertIn("immutable", failures[0])

    def test_latch_existing_at_base_cannot_be_deleted(self):
        self.write(POLICY.LATCH, '{"tag":"v0.1.0"}\n')
        self.base = self.commit("latched base")
        (self.root / POLICY.LATCH).unlink()
        self.git("add", "-u")
        failures = self.evaluate(self.commit("delete latch"))
        self.assertIn("immutable", failures[0])


if __name__ == "__main__":
    unittest.main()
