#!/usr/bin/env bash
# Contractual-docs check (CI-enforced). For every .rs file:
#   1. a companion .md exists (same path, .rs -> .md)
#   2. the companion contains a '## Graph context' section
#   3. the .rs carries at least one '// md:' marker, and every marker appears in
#      the companion
#   4. no marker is duplicated within the .rs (RULE 4: one marker per block)
#   5. the companion has a '## Coverage checklist' whose row count equals the
#      marker count (one row per block — grouped rows like "| 5-17 |" fail)
#   6. no elision pattern appears inside any ```rust fence of the companion
#      (// ..., // snip, // rest unchanged, /* ... */)
#   7. the .rs carries no comment lines other than '// md:' markers
#      (uncommented-code convention: explanation lives in the companion).
#      Caveat: the trailing-comment pattern can false-positive on a string
#      literal containing ' // ' — reword the string if that ever fires.
#
# What this script does NOT verify (agent/human duty, HARD RULE 7): that each
# fence's content is character-for-character identical to the marked block in
# the .rs. Marker/row correspondence is mechanical; verbatim fidelity is the
# author's self-check.
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
err() { echo "$1"; fail=1; }

while IFS= read -r -d '' rs; do
  md="${rs%.rs}.md"

  # 1. companion exists
  if [[ ! -f "$md" ]]; then
    err "MISSING companion doc: $md (for $rs)"
    continue
  fi

  # 2. Graph context section
  grep -q '^## Graph context' "$md" \
    || err "MISSING '## Graph context' section in $md"

  # 3+4. markers: present, unique, mirrored in the companion
  total_markers=$(grep -cE '^[[:space:]]*// md:' "$rs" || true)
  if [[ "$total_markers" -eq 0 ]]; then
    err "NO '// md:' markers in $rs (unmigrated? every block needs exactly one)"
  fi
  dups=$(grep -oE '// md:.+$' "$rs" | sed 's/[[:space:]]*$//' | sort | uniq -d)
  [[ -n "$dups" ]] && err "DUPLICATE markers in $rs: $dups"

  while IFS= read -r m; do
    [[ -n "$m" ]] || continue
    grep -qF -- "$m" "$md" \
      || err "MARKER missing from companion: '$m' ($rs -> $md)"
  done < <(grep -oE '// md:.+$' "$rs" | sed 's/[[:space:]]*$//' | sort -u)

  # 5. Coverage checklist: exists, one row per marker
  if grep -q '^## Coverage checklist' "$md"; then
    rows=$(awk '/^## Coverage checklist/{f=1;next} /^## /{f=0} f && /^\|[[:space:]]*[0-9]/' "$md" | wc -l | tr -d ' ')
    if [[ "$rows" -ne "$total_markers" ]]; then
      err "CHECKLIST row count ($rows) != marker count ($total_markers) in $md (one row per block; grouped rows are not allowed)"
    fi
  else
    err "MISSING '## Coverage checklist' section in $md"
  fi

  # 6. no elision inside ```rust fences
  if awk '/^```rust/{inf=1;next} /^```/{inf=0} inf' "$md" \
     | grep -qE '//[[:space:]]*\.\.\.|//[[:space:]]*(snip|rest unchanged|as before)|/\*[[:space:]]*\.\.\.[[:space:]]*\*/'; then
    err "ELISION pattern inside a \`\`\`rust fence in $md:"
    awk '/^```rust/{inf=1;next} /^```/{inf=0} inf' "$md" \
      | grep -nE '//[[:space:]]*\.\.\.|//[[:space:]]*(snip|rest unchanged|as before)|/\*[[:space:]]*\.\.\.[[:space:]]*\*/' || true
  fi

  # 7. uncommented-code convention: the only comment lines allowed in the .rs
  #    are '// md:' markers (all explanation lives in the companion)
  bad_comments=$( { grep -nE '^[[:space:]]*(//|/\*)' "$rs"; \
                    grep -nE '[[:alnum:];)}][[:space:]]+//' "$rs"; } \
                  | grep -vE '// md:' | sort -t: -k1 -n -u || true)
  if [[ -n "$bad_comments" ]]; then
    err "COMMENT lines that are not '// md:' markers in $rs (explanation lives in the companion .md):"
    echo "$bad_comments"
  fi
done < <(find . \
  -path ./target -prune -o \
  -path ./graphify-out -prune -o \
  -path ./.git -prune -o \
  -name '*.rs' -print0)

if [[ $fail -ne 0 ]]; then
  echo
  echo "Every .rs needs a companion .md in the block-complete format:"
  echo "docs/templates/source-module.md (v2.3). See its 9 HARD RULES."
  exit 1
fi
echo "docs check OK: companions, markers, checklists and fences all consistent"
