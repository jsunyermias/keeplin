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
| `test_nested_fenced_code_bypass_does_not_substitute_for_anchor` | `1` | the old declared-subset nested-fence bypass is red on the old checker |
| `test_link_title_bypass_does_not_substitute_for_anchor` | `1` | the old declared-subset link-title bypass is red on the old checker |
| `test_a_missing_surface_fails_closed` | `1` | absence is never skipped |
| `test_every_surface_is_checked_independently` | `1` | every enrolled path is required independently |
| `test_enrolment_remains_the_same_fixed_three_surfaces` | `0` | an unenrolled `CHANGELOG.md` does not widen the whitelist |
| `test_changelog_names_the_real_bounded_history_checker` | n/a | documentation names the checked-in `.py` executable |

The two bypass fixtures include both the new anchor and old canonical prose on unaffected
surfaces. Only the attacked surface lacks the anchor. That construction prevents a failure on a
different surface from masking whether the old prose checker accepts the hidden sentence.

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
the two old bypass regressions are measured without editing the old implementation.

## Run

```sh
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

## Related files

- `scripts/check-bounded-history.py` — checker under test.
- `scripts/check-bounded-history.py.md` — exact line rule and fixed enrolment.
- `scripts/check-docs.sh` — invokes the checker as check 12.
