# `scripts/check-bounded-history.py` — require the journal bound in reader-visible prose

## Purpose

The review journal's guarantee is easy to state as more than it is. Its unkeyed chain detects
accidental corruption and casual editing that does not rebuild the digests; it does not
authenticate records against another repository workflow carrying the same App identity.
Deletion is detected only when a surviving descendant commits to the missing record. It does
not detect terminal truncation: an actor with repository write access can delete the newest
records, and the shorter prefix evaluates as though those rounds never happened.

That matters beyond history-keeping. The deleted records can be the ones establishing that a
finding was reified, and once that evidence is gone the finding can be filed `advisory` and
converge without the authorization the surviving records would have demanded. This checker
requires the exact bounded-history sentence that states both the limitation and its consequence.

## What it checks

Three manually enrolled surfaces must each carry one canonical sentence, verbatim:

- `AGENTS.md`
- `.github/scripts/README.md`
- `docs/review-stalls.md`

This is a fixed whitelist. Enrolment is manual, so a new document that states the guarantee is
not discovered or checked until a maintainer adds it to `SURFACES`. The checker never infers
enrolment from prose.

> Terminal truncation is not detected: it can erase the record that established reification,
> after which the shorter authentic prefix may converge with that finding advisory.

After selecting reader-visible text with the declared subset below, the checker collapses
whitespace so ordinary line wrapping does not alter the contract. Every other character of the
canonical sentence must be present exactly. A missing or unreadable surface fails closed.

## Declared Markdown subset

The implementation uses `companion_tool.reader_visible_markdown`, sharing the fenced- and
indented-code state machine that was developed for the review-debt registry instead of carrying
a second version. It recognizes and ignores:

- backtick and tilde fenced code at the document margin or after one blockquote marker, with up
  to three leading spaces and a closing fence using the same marker at least as long as the
  opener;
- blank-line-delimited code indented by four spaces or one tab;
- HTML comments outside code, including comments spanning lines;
- same-line backtick code spans, so a literal `<!--` in inline code cannot open a comment;
- simple single-line link-reference definitions with a nonempty label and destination. Every
  recognized definition is ignored; the checker does not try to decide whether another link
  uses it.

This is intentionally not a CommonMark parser. The following constructs are not handled:

- blockquotes deeper than one level;
- list-item continuation indentation and code blocks nested in list items;
- raw inline or block HTML other than `<!-- -->` comments;
- reference definitions split across lines;
- inline-code spans split across lines;
- inline link or image destinations and titles.

A canonical sentence hidden in any construct on that list is neither guaranteed to count nor
guaranteed to be ignored. The tests pin the current result for one example of each construct;
that pin makes a later behavior change explicit, but it does not expand the declared grammar.

## Why verbatim, and not the words it contains

The first implementation required only the substrings `truncat`, `reifi` and `advisory`
somewhere in each file. Deleting the bounded-history paragraphs and leaving `Glossary:
truncation, reification, advisory.` kept that check green. The exact sentence is therefore the
contract: weakening `is not detected` or dropping the consequence after the colon fails.

## Failure behavior and usage

The checker uses only Python's standard library and returns `0` when all three surfaces pass,
`1` when a surface is absent, unreadable or lacks the visible sentence, and `2` for invalid
command-line usage. It prints every failing surface before the canonical sentence.

```sh
./scripts/check-bounded-history.py            # repository root inferred from the script
./scripts/check-bounded-history.py /some/root # fixture root used by tests
```

`scripts/check-docs.sh` invokes it directly as check 12 and preserves that exit-status contract.

## Graph context

LAYER 1 (Graphify) locates this script among the repository's governance checks. LAYER 2 is
this companion. The rule it enforces follows keeplin ADR 0008's bounded history claim. No local
graph artifact was available while this implementation was prepared, so these relationships are
authored inference rather than refreshed extracted edges.

## Related files

- `scripts/companion_tool.py` — owns the reusable declared Markdown subset.
- `scripts/check-docs.sh` — runs this checker as check 12.
- `scripts/tests/test_bounded_history.py` — positive, negative, regression and subset-boundary
  tests.
- `docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md` — accepted
  decision that bounds the claim; unchanged by this task.
- `.github/scripts/check-review-loop.js` — evaluator whose bounded history is being stated.
- `docs/review-stalls.md` — one of the three manually enrolled surfaces.
