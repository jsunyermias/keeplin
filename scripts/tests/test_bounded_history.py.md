# `scripts/tests/test_bounded_history.py` — exact-anchor regressions

## What is tested

The tests execute the real `scripts/check-bounded-history.py` against temporary roots containing
the same three manually enrolled paths as the repositories.

| Test | Expected | Contract |
|------|----------|----------|
| `test_the_real_repository_passes` | `0` | all checked-in enrolled surfaces carry the anchor |
| `test_intact_fixtures_pass` | `0` | a fixture with one exact standalone anchor per surface passes |
| `test_canonical_sentence_without_anchor_fails` | `1` | old prose, even semantically correct prose, is not the machine anchor |
| `test_anchor_must_be_one_exact_standalone_line` | `1` | embedding the anchor in a longer line does not satisfy equality |
| `test_near_match_does_not_satisfy_the_anchor` | `1` | wording changes are explicit contract changes |
| `test_anchor_hidden_in_multiline_html_comment_counts_as_raw_enrolment` | `0` | the raw-line checker does not enforce rendered visibility through HTML comments |
| `test_anchor_inside_fenced_code_counts_as_raw_enrolment` | `0` | the raw-line checker does not enforce whether the anchor is ordinary policy prose |
| `test_a_missing_surface_fails_closed` | `1` | absence is never skipped |
| `test_every_surface_is_checked_independently` | `1` | every enrolled path is required independently |
| `test_enrolment_remains_the_same_fixed_three_surfaces` | `0` | an unenrolled `CHANGELOG.md` does not widen the whitelist |
| `test_changelog_names_the_real_bounded_history_checker` | n/a | documentation names the checked-in `.py` executable |

The two concealment fixtures put the actual anchor line inside a multi-line HTML comment and an
ordinary margin fence. Both pass deliberately: the decided interim contract is raw enrolment,
not Markdown parsing or rendered conspicuousness. If the maintainer later chooses visibility
enforcement, these focused fixtures flip to exit `1` without changing what they exercise.

## Retired parser tests

The former suite tested whitespace collapsing and a declared Markdown subset: headings,
blockquotes, backtick and tilde fences, indentation, HTML comments, inline code, reference
definitions, fence lengths and markers, raw HTML, nested list code, multiline definitions, and
link titles. Those tests were removed because the checker no longer selects reader-visible prose
or implements any Markdown state machine. Keeping them would specify behavior the decided anchor
checker neither has nor needs. Missing-surface, per-surface enrolment, real-repository, executable
name, and positive-fixture coverage remain because those contracts did not change.

## Harness and old-code measurement

`BOUNDED_HISTORY_CHECK` points the suite at a preserved checker in a scratch worktree. This is how
the concealment contract is measured without editing the old implementation.

## Run

```sh
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

## Related files

- `scripts/check-bounded-history.py` — checker under test.
- `scripts/check-bounded-history.py.md` — exact line rule and fixed enrolment.
- `scripts/check-docs.sh` — invokes the checker as check 12.
