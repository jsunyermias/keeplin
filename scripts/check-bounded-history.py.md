# `scripts/check-bounded-history.py` — require the bounded-history anchor

## Purpose

The review journal cannot detect terminal truncation, which can erase the record that established
reification and allow the shorter prefix to converge with the finding advisory. The three policy
surfaces must keep that bound conspicuous without asking a checker to infer meaning from Markdown.

## Machine-checkable rule

Each manually enrolled surface must contain this exact standalone line:

> Bounded-history anchor: terminal truncation can erase reification history and enable advisory convergence.

The checker reads UTF-8 text and tests raw line equality. It does not collapse whitespace, search
for related vocabulary, interpret prose, or parse Markdown containers. A line either equals the
anchor or it does not. The anchor itself names the limitation and its consequence, so it remains
understandable to a reader as well as unambiguous to the machine.

The manually enrolled surfaces remain exactly:

- `AGENTS.md`
- `.github/scripts/README.md`
- `docs/review-stalls.md`

Enrolment remains a fixed whitelist. A new document is not discovered from its contents and does
not enter the check until a maintainer changes `SURFACES` explicitly. `CHANGELOG.md` remains
outside the whitelist.

## Failure behavior and usage

The checker uses only Python's standard library. It returns `0` when all three surfaces carry the
anchor, `1` when a surface is missing, unreadable, or lacks the exact line, and `2` for invalid
command-line usage. It reports every failing surface and prints the required anchor.

```sh
./scripts/check-bounded-history.py
./scripts/check-bounded-history.py /some/fixture/root
```

`scripts/check-docs.sh` invokes it as check 12.

## Graph context

LAYER 1 (Graphify) locates this script among the repository's governance checks. LAYER 2 is this
companion. The rule follows keeplin ADR 0008's bounded-history claim. No local graph artifact was
available, so these relationships are authored inference rather than refreshed extracted edges.

## Related files

- `scripts/check-docs.sh` — runs the checker.
- `scripts/tests/test_bounded_history.py` — exact-anchor, enrolment, failure, and old-bypass tests.
- `AGENTS.md`, `.github/scripts/README.md`, `docs/review-stalls.md` — the unchanged three-surface
  enrolment.
- `docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md` — accepted
  decision whose terminal-truncation bound the anchor keeps visible.
