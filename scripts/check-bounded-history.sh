#!/usr/bin/env bash
# md:check-bounded-history
# Every surface that states the review journal's guarantee must also state its bound, in one
# canonical sentence, verbatim. The guarantee is not "history cannot be rewritten": terminal
# truncation is undetected, and a truncated prefix can erase the record establishing that a
# finding was reified and let it converge as advisory.
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

fail=0
for surface in "${SURFACES[@]}"; do
  path="$root/$surface"
  if [[ ! -f $path ]]; then
    echo "BOUNDED HISTORY: $surface is missing from $root"
    fail=1
    continue
  fi
  # Line wrapping is a formatting choice, so whitespace is collapsed before matching; every
  # other byte of the sentence must be present exactly.
  if ! tr '\n' ' ' <"$path" | tr -s '[:space:]' ' ' | grep -qF -- "$CANONICAL"; then
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
