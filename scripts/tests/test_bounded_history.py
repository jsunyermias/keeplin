"""Mutation tests for the bounded-history anchor checker."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CHECK = Path(
    os.environ.get("BOUNDED_HISTORY_CHECK", REPO / "scripts" / "check-bounded-history.py")
).resolve()
SURFACES = ("AGENTS.md", ".github/scripts/README.md", "docs/review-stalls.md")
ANCHOR = (
    "Bounded-history anchor: terminal truncation can erase reification history and "
    "enable advisory convergence."
)
CANONICAL = (
    "Terminal truncation is not detected: it can erase the record that established "
    "reification, after which the shorter authentic prefix may converge with that finding "
    "advisory."
)


def run(root):
    return subprocess.run(
        [str(CHECK), str(root)], capture_output=True, text=True, cwd=REPO
    )


class BoundedHistoryCheck(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        for surface in SURFACES:
            path = self.root / surface
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"# {surface}\n\n{ANCHOR}\n", encoding="utf-8")

    def write(self, surface, text):
        (self.root / surface).write_text(text, encoding="utf-8")

    def test_the_real_repository_passes(self):
        result = run(REPO)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_intact_fixtures_pass(self):
        result = run(self.root)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_canonical_sentence_without_anchor_fails(self):
        self.write("AGENTS.md", f"# agents\n\n{CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_anchor_must_be_one_exact_standalone_line(self):
        self.write("AGENTS.md", f"# agents\n\nPrefix {ANCHOR}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_near_match_does_not_satisfy_the_anchor(self):
        weakened = ANCHOR.replace("can erase", "might erase")
        self.write(".github/scripts/README.md", f"# readme\n\n{weakened}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_anchor_hidden_in_multiline_html_comment_counts_as_raw_enrolment(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n<!--\n{ANCHOR}\n-->\n",
        )
        self.assertEqual(run(self.root).returncode, 0)

    def test_anchor_inside_fenced_code_counts_as_raw_enrolment(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n```text\n{ANCHOR}\n```\n",
        )
        self.assertEqual(run(self.root).returncode, 0)

    def test_a_missing_surface_fails_closed(self):
        (self.root / "AGENTS.md").unlink()
        result = run(self.root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing", result.stdout)

    def test_every_surface_is_checked_independently(self):
        for surface in SURFACES:
            with self.subTest(surface=surface):
                self.setUp()
                self.write(surface, "# gutted\n\nNothing to see here.\n")
                result = run(self.root)
                self.assertEqual(result.returncode, 1)
                self.assertIn(surface, result.stdout)

    def test_enrolment_remains_the_same_fixed_three_surfaces(self):
        (self.root / "CHANGELOG.md").write_text("# no anchor here\n", encoding="utf-8")
        result = run(self.root)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_changelog_names_the_real_bounded_history_checker(self):
        changelog = (REPO / "CHANGELOG.md").read_text(encoding="utf-8")
        self.assertIn("scripts/check-bounded-history.py", changelog)
        self.assertNotIn("scripts/check-bounded-history.sh", changelog)


if __name__ == "__main__":
    unittest.main()
