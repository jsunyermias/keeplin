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
