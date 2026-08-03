# `scripts/check-bounded-history.sh` — the journal guarantee must carry its bound

## Complete source

```bash
# md:check-bounded-history
# Three manually enrolled policy surfaces must state the review journal's bound in one canonical
# sentence, verbatim. Enrolment is intentionally not inferred from prose: a new surface is not
# checked until it is added to SURFACES. The guarantee is not "history cannot be rewritten":
# terminal truncation is undetected, and a truncated prefix can erase the record establishing
# that a finding was reified and let it converge as advisory.
#
# The check is deliberately a verbatim match on a fixed sentence rather than a search for the
# words it contains. An earlier version required only the substrings "truncat", "reifi" and
# "advisory" somewhere in each file, which a glossary line satisfies while the bounded-history
# paragraphs are deleted — a check that looks like evidence and is not.
#
# Usage: check-bounded-history.sh [root]   (default: the repository root)
set -uo pipefail
root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"

CANONICAL="Terminal truncation is not detected: it can erase the record that established reification, after which the shorter authentic prefix may converge with that finding advisory."
SURFACES=(AGENTS.md .github/scripts/README.md docs/review-stalls.md)

visible_prose() {
  awk '
    function fence_width(text, marker, count) {
      count = 0
      while (substr(text, count + 1, 1) == marker) count++
      return count
    }
    function strip_html(text, start, finish, before, after) {
      while (1) {
        if (in_html) {
          finish = index(text, "-->")
          if (!finish) return ""
          text = substr(text, finish + 3)
          in_html = 0
        }
        start = index(text, "<!--")
        if (!start) return text
        before = substr(text, 1, start - 1)
        after = substr(text, start + 4)
        finish = index(after, "-->")
        if (!finish) {
          in_html = 1
          return before
        }
        text = before substr(after, finish + 3)
      }
    }
    {
      line = $0
      if (in_fence) {
        candidate = line
        sub(/^ {0,3}/, "", candidate)
        if (substr(candidate, 1, fence_length) == fence_run) {
          rest = substr(candidate, fence_length + 1)
          if (rest ~ /^[[:space:]]*$/) in_fence = 0
        }
        next
      }
      candidate = line
      sub(/^ {0,3}/, "", candidate)
      marker = substr(candidate, 1, 1)
      if ((marker == "`" || marker == "~") && fence_width(candidate, marker) >= 3) {
        fence_length = fence_width(candidate, marker)
        fence_run = substr(candidate, 1, fence_length)
        in_fence = 1
        next
      }
      if (line ~ /^(    |\t)/) next
      print strip_html(line)
    }
  '
}

fail=0
for surface in "${SURFACES[@]}"; do
  path="$root/$surface"
  if [[ ! -f $path ]]; then
    echo "BOUNDED HISTORY: $surface is missing from $root"
    fail=1
    continue
  fi
  # Comments and code examples are not statements a prose reader is asked to rely on. Remove
  # them, then collapse whitespace so line wrapping remains only a formatting choice.
  if ! visible_prose <"$path" | tr '\n' ' ' | tr -s '[:space:]' ' ' | grep -qF -- "$CANONICAL"; then
    echo "BOUNDED HISTORY: $surface does not carry the canonical bounded-history sentence verbatim"
    fail=1
  fi
done

if [[ $fail -ne 0 ]]; then
  echo
  echo "The canonical sentence is:"
  echo "  $CANONICAL"
  exit 1
fi
echo "bounded-history check OK: ${#SURFACES[@]} surfaces carry the canonical sentence"
```

## Purpose

The review journal's guarantee is easy to state as more than it is. Its unkeyed chain detects
accidental corruption and casual editing that does not rebuild the digests; it does not
authenticate records against another repository workflow carrying the same App identity.
Deletion is detected only when a surviving descendant commits to the missing record. It does
**not** detect terminal truncation: an actor with repository write access can delete the newest
records, and the shorter prefix evaluates as though those rounds never happened.

That matters beyond history-keeping. The deleted records can be the ones establishing that a
finding was reified, and once that evidence is gone the finding can be filed `advisory` and
converge — without the authorization the surviving records would have demanded.

Prose that states the guarantee without that bound is the defect this whole change exists to
remove: an unconditional promise that in fact holds only if something else holds. It recurred
five times across nine review rounds, each time caught by a human or a model reading carefully.
This check stops relying on that.

## What it checks

Three manually enrolled surfaces must each carry one canonical sentence, verbatim:

- `AGENTS.md`
- `.github/scripts/README.md`
- `docs/review-stalls.md`

This is a fixed whitelist. Enrolment is manual, so a new document that states the guarantee is not
discovered or checked until a maintainer adds it to `SURFACES`. That narrower rule is deliberate:
reliably inferring the meaning of arbitrary prose would create another check that looks stronger
than the evidence it actually supplies.

> Terminal truncation is not detected: it can erase the record that established reification,
> after which the shorter authentic prefix may converge with that finding advisory.

HTML comments, fenced code blocks and four-space or tab-indented code blocks are removed before
matching, because an occurrence hidden there is an example or metadata rather than a statement to
the reader. Line wrapping is a formatting choice, so the remaining prose is flattened and its
whitespace collapsed before matching. Every other byte must be present exactly. A missing surface
fails closed.

## Why verbatim, and not the words it contains

The first implementation of this rule required only that each file contain the substrings
`truncat`, `reifi` and `advisory` somewhere. Round 10 of independent review demonstrated that
this is satisfied by deleting the bounded-history paragraphs and leaving a line reading
`Glossary: truncation, reification, advisory.` — the check stays green while the guarantee it
was protecting is gone. A check that cannot fail on the defect it names is worse than no check,
because it is quoted as evidence.

A verbatim sentence is blunt and it is a real constraint: weakening `is not detected` to
`may not always be detected`, or dropping the consequence after the colon, fails.
`scripts/tests/test_bounded_history.py` asserts each of those mutations, so the check's own
ability to fail is itself tested.

## Usage

```sh
./scripts/check-bounded-history.sh            # the repository root
./scripts/check-bounded-history.sh /some/root # a fixture tree, used by the tests
```

`scripts/check-docs.sh` runs it as check 12 and folds its exit status into the docs gate, so
CI fails on a weakened surface.

## Graph context

LAYER 1 (Graphify) locates this script among the repository's governance checks. LAYER 2 is
this companion. The rule it enforces is a consequence of keeplin ADR 0008, which states the
bounded history claim rather than making a fifth attempt to engineer around it.

## Related files

- `scripts/check-docs.sh` — runs this as check 12.
- `scripts/tests/test_bounded_history.py` — mutation tests proving the check can fail.
- `docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md` — the
  accepted decision that bounds the claim.
- `.github/scripts/check-review-loop.js` — the evaluator whose declassification protection this
  bound qualifies.
- `docs/review-stalls.md` — one of the three checked surfaces, and the stall protocol itself.
