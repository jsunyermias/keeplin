#!/usr/bin/env bash
# md:check-docs
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
#   8. every Rust fence maps 1:1 to a source leaf and is identical after LF-only
#      line-ending normalization; stale, truncated, orphan and duplicate fences fail,
#      and so does unmarked code between a container marker and its first child
#      or before the first marker; write-mode sync preserves bytes outside valid fences
#   9. supported shell sources use exact '# md:'/bash pairs, every SQL migration has
#      one complete verbatim sql fence, unsupported formats are ignored, and sync never
#      writes source SQL or bytes outside valid companion fences.
#  10. the generated context manifest is current.
#  11. review-ledger rows use exactly one of the four states defined by AGENTS.md.
#  12. every surface stating the journal guarantee also states its bound: terminal
#      truncation, reification, and the advisory consequence of losing that record.
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

# 8+9. Fidelity is mechanical for Rust, supported shell sources and SQL migrations.
#      The synchronizer's check mode performs no writes and uses only Python's standard
#      library; all existing Rust structural checks above remain intentionally redundant.
if ! ./scripts/sync-companion-code --check; then
  fail=1
fi

# 10. Context provenance and pack routing must describe the current companions.
if ! ./scripts/context-pack manifest --check; then
  fail=1
fi

while IFS= read -r row; do
  state=$(awk -F'|' '{ value=$5; gsub(/^[[:space:]]+|[[:space:]]+$/, "", value); print tolower(value) }' <<<"$row")
  case "$state" in
    open|resolved|dismissed|advisory) ;;
    *) err "INVALID review-ledger state '$state' (allowed: open, resolved, dismissed, advisory): $row" ;;
  esac
done < <(grep -RhsE '^\|[[:space:]]*F-[0-9]{3,}[[:space:]]*\|' --include='*.md' . \
  --exclude-dir=.git --exclude-dir=graphify-out --exclude-dir=target || true)

# 12. Every surface that states the journal's guarantee must also state its bound. The
#     guarantee is not "history cannot be rewritten": terminal truncation is undetected, and
#     the consequence is that a truncated prefix can erase the record establishing that a
#     finding was reified and let it converge as advisory. A surface that promises the first
#     without the second is the unconditional-promise defect this change exists to remove.
for surface in AGENTS.md .github/scripts/README.md docs/review-stalls.md; do
  [[ -f $surface ]] || continue
  for token in truncat reifi advisory; do
    grep -qis "$token" "$surface" \
      || err "BOUNDED HISTORY: $surface states the journal guarantee without '$token' — it must state terminal truncation, reification and the advisory consequence together"
  done
done

if [[ $fail -ne 0 ]]; then
  echo
  echo "Every supported source needs a mechanically faithful companion:"
  echo "docs/templates/source-module.md (v2.5). See its 9 HARD RULES."
  exit 1
fi
echo "docs check OK: structure, exact fences and context manifest are consistent"
