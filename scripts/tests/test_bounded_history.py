"""Mutation tests for scripts/check-bounded-history.sh.

The check exists because prose that states the journal's guarantee without its bound is the
defect the review loop keeps producing. A check that cannot fail is worse than no check, so
these tests gut the fixtures in the ways a careless edit or a deliberate weakening would and
require a non-zero exit for each.
"""

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CHECK = REPO / "scripts" / "check-bounded-history.sh"
SURFACES = ("AGENTS.md", ".github/scripts/README.md", "docs/review-stalls.md")
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
            path.write_text(f"# {surface}\n\nSome prose. {CANONICAL} More prose.\n")

    def write(self, surface, text):
        (self.root / surface).write_text(text)

    def test_the_real_repository_passes(self):
        self.assertEqual(run(REPO).returncode, 0, run(REPO).stdout)

    def test_intact_fixtures_pass(self):
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_line_wrapping_does_not_matter(self):
        wrapped = CANONICAL.replace(" that ", "\nthat ").replace(", after", ",\nafter")
        self.write("AGENTS.md", f"# wrapped\n\n{wrapped}\n")
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_a_glossary_of_the_words_does_not_satisfy_the_check(self):
        # The exact defect the previous substring implementation allowed: the bounded-history
        # prose is deleted, the vocabulary survives somewhere else in the file, check passes.
        self.write(
            "docs/review-stalls.md",
            "# stalls\n\nGlossary: truncation, reification, advisory.\n",
        )
        result = run(self.root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("docs/review-stalls.md", result.stdout)

    def test_a_weakened_sentence_does_not_satisfy_the_check(self):
        # "may be able to" is exactly the hedge that turns a stated limit back into a promise.
        weakened = CANONICAL.replace("is not detected", "may not always be detected")
        self.write(".github/scripts/README.md", f"# readme\n\n{weakened}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_dropping_the_consequence_does_not_satisfy_the_check(self):
        truncated = CANONICAL.split(":")[0] + "."
        self.write("AGENTS.md", f"# agents\n\n{truncated}\n")
        self.assertEqual(run(self.root).returncode, 1)

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


if __name__ == "__main__":
    unittest.main()
