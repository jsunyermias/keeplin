# `scripts/check-review-debt.sh` — cap on open review debt

## Complete source

```bash
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
```

## Purpose

A maintainer waiver defers an independent review; it does not cancel it. Nothing stopped the
deferred reviews from accumulating: each waiver was individually recorded and individually
reasonable, and the backlog grew anyway. This check makes the backlog cost something. Past the
cap, `Check, Test & Lint` fails and the repository stops accepting new code until entries are
paid down.

The cap is a forcing function, not a judgement about any single entry. It converts "we will get
to it" into "we get to it before the next change lands".

## What it counts

Data rows under `## Open` in `docs/review-debt.md`. Deliberately excluded:

- the column header row, which is the first `|` row of the section;
- the `|---|---|` alignment separator, in any of its `:--:` forms;
- the all-em-dash placeholder row that marks a genuinely empty section;
- everything inside a fenced code block, so an illustrative table in the prose above the
  registry never inflates the count.

`## Cleared` is not counted. Clearing an entry is exactly how the count goes down.

## The cap

`MAX_OPEN_REVIEW_DEBT` sets it; the default is 2. Raising it is a deliberate act and belongs in
the pull request that raises it, with its reason. Raising it silently defeats the check.

## Failure behavior

Above the cap the script prints the count, the limit and the two legitimate ways out — clear
entries, or raise the cap on purpose — and exits 1. A missing registry is also a failure: the
absence of the file is not the absence of debt.

## Portability

The counter is `awk` without interval expressions and without literal multibyte characters (the
em dash is matched by its UTF-8 byte sequence). It behaves identically under `mawk` and `gawk`,
so CI and a contributor's machine agree.

## Related files

- `docs/review-debt.md` — the registry this reads.
- `check-docs.sh` — verifies the registry's shape; this script bounds its size. Shape and size
  are different properties and are checked separately on purpose.
- `.github/scripts/check-review-governance.js` — the pull-request side: what may be waived, and
  what a waiver must record.
