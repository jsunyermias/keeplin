"""Mutation tests for scripts/check-bounded-history.py.

The check exists because prose that states the journal's guarantee without its bound is the
defect the review loop keeps producing. A check that cannot fail is worse than no check, so
these tests gut the fixtures in the ways a careless edit or a deliberate weakening would and
require a non-zero exit for each.
"""

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

    def test_canonical_sentence_cannot_be_synthesized_across_markdown_blocks(self):
        first, second = CANONICAL.split(" reification", 1)
        self.write("AGENTS.md", f"# agents\n\n{first}\n> reification{second}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_an_html_comment_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n<!-- {CANONICAL} -->\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_fenced_code_block_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n```text\n{CANONICAL}\n```\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_tilde_fenced_code_block_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n~~~text\n{CANONICAL}\n~~~\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_fence_with_three_leading_spaces_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n   ```text\n{CANONICAL}\n   ```\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_an_indented_code_block_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n    {CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_an_indented_code_block_at_document_start_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"    {CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_indented_code_after_a_fence_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"```text\nexample\n```\n    {CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_margin_prose_after_a_fence_still_satisfies_the_check(self):
        self.write("AGENTS.md", f"```text\nexample\n```\n{CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_indented_code_after_a_heading_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# heading\n    {CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_margin_prose_after_a_heading_still_satisfies_the_check(self):
        self.write("AGENTS.md", f"# heading\n{CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_a_tab_indented_code_block_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n\t{CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_fence_inside_one_blockquote_does_not_satisfy_the_check(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n> ```text\n> {CANONICAL}\n> ```\n",
        )
        self.assertEqual(run(self.root).returncode, 1)

    def test_unquoted_line_after_a_blockquote_fence_is_visible_prose(self):
        self.write("AGENTS.md", f"> ```text\n{CANONICAL}\n")
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_lazy_continuation_of_a_blockquote_paragraph_stays_in_one_block(self):
        first, second = CANONICAL.split(" reification", 1)
        self.write("AGENTS.md", f"> {first}\nreification{second}\n")
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_a_blockquote_fence_cannot_close_a_margin_fence(self):
        self.write(
            "AGENTS.md",
            f"```text\nexample\n> ```\n{CANONICAL}\n```\n",
        )
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_margin_fence_cannot_close_a_blockquote_fence(self):
        self.write(
            "AGENTS.md",
            f"> ```text\n> example\n```\n> {CANONICAL}\n> ```\n",
        )
        self.assertEqual(run(self.root).returncode, 1)

    def test_an_unused_link_reference_definition_does_not_satisfy_the_check(self):
        self.write(
            "AGENTS.md",
            f'# agents\n\n[unused]: https://example.invalid "{CANONICAL}"\n',
        )
        self.assertEqual(run(self.root).returncode, 1)

    def test_link_reference_definition_with_an_empty_label_remains_visible(self):
        self.write(
            "AGENTS.md",
            f'# agents\n\n[]: https://example.invalid "{CANONICAL}"\n',
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_link_reference_definition_with_an_empty_destination_remains_visible(self):
        self.write(
            "AGENTS.md",
            f'# agents\n\n[unused]: <> "{CANONICAL}"\n',
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_a_fence_inside_an_html_comment_does_not_hide_later_prose(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n<!--\n```text\n-->\n\n{CANONICAL}\n",
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_a_multiline_html_comment_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n<!--\n{CANONICAL}\n-->\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_longer_closing_fence_exposes_later_prose(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n```text\nexample\n````\n\n{CANONICAL}\n",
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_an_html_comment_marker_inside_inline_code_does_not_hide_later_prose(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\nThe token `<!--` is shown literally.\n\n{CANONICAL}\n",
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_same_line_inline_code_does_not_satisfy_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n`{CANONICAL}`\n")
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_shorter_fence_does_not_close_a_longer_fence(self):
        self.write(
            "AGENTS.md",
            f"````text\nexample\n```\n{CANONICAL}\n````\n",
        )
        self.assertEqual(run(self.root).returncode, 1)

    def test_a_different_fence_marker_does_not_close_a_fence(self):
        self.write(
            "AGENTS.md",
            f"```text\nexample\n~~~\n{CANONICAL}\n```\n",
        )
        self.assertEqual(run(self.root).returncode, 1)

    def test_normal_prose_still_satisfies_the_check(self):
        self.write("AGENTS.md", f"# agents\n\n{CANONICAL}\n")
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

    def test_changelog_names_the_real_bounded_history_checker(self):
        changelog = (REPO / "CHANGELOG.md").read_text()
        self.assertIn("scripts/check-bounded-history.py", changelog)
        self.assertNotIn("scripts/check-bounded-history.sh", changelog)

    def test_deeper_blockquote_fences_pin_out_of_subset_behavior(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n> > ```text\n> > {CANONICAL}\n> > ```\n",
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_list_nested_fences_pin_out_of_subset_behavior(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n- ```text\n  {CANONICAL}\n  ```\n",
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_raw_html_blocks_pin_out_of_subset_behavior(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n<script type=\"text/plain\">\n{CANONICAL}\n</script>\n",
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_multiline_reference_definitions_pin_out_of_subset_behavior(self):
        self.write(
            "AGENTS.md",
            f'# agents\n\n[unused]:\n  https://example.invalid\n  "{CANONICAL}"\n',
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_multiline_inline_code_spans_pin_out_of_subset_behavior(self):
        self.write(
            "AGENTS.md",
            f"# agents\n\n`not prose\n{CANONICAL}\nstill code`\n",
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)

    def test_inline_link_titles_pin_out_of_subset_behavior(self):
        self.write(
            "AGENTS.md",
            f'# agents\n\n[policy](https://example.invalid "{CANONICAL}")\n',
        )
        self.assertEqual(run(self.root).returncode, 0, run(self.root).stdout)


if __name__ == "__main__":
    unittest.main()
