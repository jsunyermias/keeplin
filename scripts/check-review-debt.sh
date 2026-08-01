#!/usr/bin/env bash
# md:check-review-debt
# Caps open review debt. A waiver defers a review; it does not cancel it. Past the
# cap, the repository stops accepting new code until the backlog is paid down.
#
# Counts data rows under '## Open' in docs/review-debt.md, excluding the column
# header, the |---| separator, the empty-section placeholder row, and anything
# inside a fenced code block. Exits 1 above MAX_OPEN_REVIEW_DEBT (default 2).
set -uo pipefail
cd "$(dirname "$0")/.."

REGISTRY="docs/review-debt.md"
MAX="${MAX_OPEN_REVIEW_DEBT:-2}"

if [[ ! -f "$REGISTRY" ]]; then
  echo "check-review-debt: MISSING $REGISTRY" >&2
  exit 1
fi

count=$(awk '
  /^[[:space:]]*```/ { infence = !infence; next }
  /^[[:space:]]*~~~/ { infence = !infence; next }
  infence            { next }

  /^##[[:space:]]+Open[[:space:]]*$/ { insec = 1; hdr = 0; next }
  /^##[[:space:]]/                   { insec = 0; next }
  !insec                             { next }
  $0 !~ /^[[:space:]]*\|/            { next }

  {
    if (hdr == 0) { hdr = 1; next }          # column header row

    line = $0
    sub(/^[[:space:]]*\|/, "", line)
    sub(/\|[[:space:]]*$/, "", line)
    n = split(line, cell, "|")

    sep = 1; filled = 0
    for (i = 1; i <= n; i++) {
      c = cell[i]
      gsub(/^[[:space:]]+/, "", c); gsub(/[[:space:]]+$/, "", c)
      if (c !~ /^:?-+:?$/) sep = 0             # |---|:--:| separator row
      if (c != "" && c != "-" && c != "\342\200\224") filled = 1   # \342\200\224 = em dash
    }
    if (sep) next
    if (!filled) next                          # all cells empty or em dash
    open++
  }
  END { print open + 0 }
' "$REGISTRY")

if (( count > MAX )); then
  echo "check-review-debt: $count open entries in $REGISTRY, limit is $MAX." >&2
  echo "A waiver defers a review, it does not cancel it. Clear entries before adding code:" >&2
  echo "move them to '## Cleared' with the review that cleared them, or raise the cap" >&2
  echo "deliberately with MAX_OPEN_REVIEW_DEBT and say so in the pull request." >&2
  exit 1
fi

echo "check-review-debt: $count open entries in $REGISTRY (limit $MAX)"
