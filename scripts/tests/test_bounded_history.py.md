# `scripts/tests/test_bounded_history.py` — the bounded-history check must be able to fail

## What is tested

`scripts/check-bounded-history.sh` enforces that three surfaces carry one canonical sentence
verbatim. A check of that kind is only worth its exit status if it fails on the defect it
names, so these tests gut fixture copies in the ways a careless edit or a deliberate weakening
would, and require a non-zero exit for each.

| Test | What it pins |
|------|--------------|
| `test_the_real_repository_passes` | the checked-in surfaces satisfy the rule right now |
| `test_intact_fixtures_pass` | the fixtures are a valid baseline, so a failure below is the mutation and not the harness |
| `test_line_wrapping_does_not_matter` | the sentence may wrap across lines; formatting is not the contract |
| `test_a_glossary_of_the_words_does_not_satisfy_the_check` | the exact hole round 10 found in the previous substring implementation: delete the bounded-history prose, leave `Glossary: truncation, reification, advisory.`, and the old check stayed green |
| `test_a_weakened_sentence_does_not_satisfy_the_check` | hedging `is not detected` into `may not always be detected` turns a stated limit back into a promise |
| `test_dropping_the_consequence_does_not_satisfy_the_check` | stating the limit without what it costs is the same defect in shorter form |
| `test_a_missing_surface_fails_closed` | a deleted surface is a failure, never a silent skip |
| `test_every_surface_is_checked_independently` | gutting any one of the three fails and names that file, so no surface rides on another |

## Fixtures

Built in a temporary directory per test, not checked in: three files, each containing the
canonical sentence surrounded by filler prose. The tests mutate copies only. The suite runs the
real script through `subprocess` against those roots, which is why the script takes a root
argument at all.

## Run

```sh
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

No network, API, Rust toolchain or model is involved.

## Related files

- `scripts/check-bounded-history.sh` — the script under test.
- `scripts/check-docs.sh` — runs it as check 12 in CI.
- `docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md` — the
  decision that bounds the claim these tests protect.
