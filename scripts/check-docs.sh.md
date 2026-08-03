# `scripts/check-docs.sh` — contractual-docs CI check

## Complete source

```bash
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
#  12. the three manually enrolled journal-policy surfaces carry the canonical bounded-history
#      sentence verbatim (delegated to scripts/check-bounded-history.sh).
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

# 12. The three manually enrolled journal-policy surfaces must state the bound verbatim.
#     Delegated so the fixed enrolment and matching rule can be exercised against fixtures.
if ! ./scripts/check-bounded-history.sh; then
  fail=1
fi

if [[ $fail -ne 0 ]]; then
  echo
  echo "Every supported source needs a mechanically faithful companion:"
  echo "docs/templates/source-module.md (v2.5). See its 9 HARD RULES."
  exit 1
fi
echo "docs check OK: structure, exact fences and context manifest are consistent"
```

## Purpose

The mechanical arbiter of the block-complete companion-doc contract (LAYER 2 of the
navigation model, `docs/templates/source-module.md` v2.5). Its structural checks use
standard shell tools and its exact-fidelity/context checks use standard-library Python, so
it still runs before any Rust toolchain exists. Run it locally before pushing; CI fails the
build on any violation.

## What it checks

For every `.rs` file in the repo (pruning `target/`, `graphify-out/`, `.git/`), it verifies:

1. **Companion exists** — a `.md` at the same path (`foo.rs` → `foo.md`).
2. **`## Graph context` section** — the companion carries it (LAYER 1 ↔ LAYER 2 link).
3. **Markers present and mirrored** — the `.rs` has at least one `// md:` marker, and
   every marker also appears verbatim in the companion (grep both directions).
4. **No duplicate markers** — each `// md:` marker occurs exactly once in the `.rs`
   (HARD RULE 4: one marker per block).
5. **Coverage-checklist correspondence** — the companion has a `## Coverage checklist`
   whose data-row count equals the marker count. Grouped rows (e.g. `| 5-17 |`) fail,
   because they collapse several blocks into one row.
6. **No elision inside `` ```rust `` fences** — the companion's rust fences contain none
   of `// ...`, `// snip`, `// rest unchanged`, `// as before`, `/* ... */` (HARD RULE 2:
   code is embedded complete, never shortened).
7. **Uncommented-code convention** — the only comment lines allowed in the `.rs` are
   `// md:` markers; any other `//`, `///`, `//!` or `/* */` comment fails (HARD RULE 9:
   all explanation lives in the companion, so fences never contain doc comments).
8. **Exact fence fidelity** — `sync-companion-code --check` maps every Rust fence to one
   source leaf and compares the full marker, attributes, signature, body and whitespace.
   Only CRLF/LF line endings are normalized; all other text must match. Stale, truncated,
   reordered, missing, orphan and duplicate fences fail (HARD RULE 7). A container's
   preamble may carry only its declaration, attributes and braces: an unmarked import,
   const or nested item before the first child marker fails as UNCOVERED (HARD RULE 6).
   Before the file's first marker, only BOM, a first-line shebang, blank lines and inner
   crate attributes are scaffolding; any Rust item is UNCOVERED. Write-mode sync replaces
   raw fence-body ranges only, preserving mixed EOL and all bytes outside valid fences.
9. **Generated context index** — `context-pack manifest --check` fails if
   `docs/context-manifest.json` no longer matches the source/companion corpus.
10. **Ledger state vocabulary** — every Markdown ledger row whose ID has the `F-001`
    shape uses exactly `open`, `resolved`, `dismissed` or `advisory`; a fifth state fails.
11. **Bounded-history consistency** — the three surfaces manually enrolled in
    `check-bounded-history.sh` (`AGENTS.md`, `.github/scripts/README.md` and
    `docs/review-stalls.md`) must each carry the canonical bounded-history sentence in
    reader-visible prose. The whitelist is fixed: a new surface is not inferred from its
    meaning and must be enrolled explicitly. This keeps the documented guarantee equal to
    the check's actual evidence.

## Known caveat

Check 7's trailing-comment pattern can false-positive on a string literal that contains
` // ` (e.g. a URL). If that ever fires on a legitimate string, reword the string — the
script is the arbiter and must not be weakened to accommodate it.

## Behaviour

- Prints one line per violation and exits `1`; prints confirmation lines and
  exits `0` when clean.
- Repo-wide: during a migration it keeps reporting not-yet-migrated files, which is
  expected. The bar for finishing one file is that *that* file produces zero violations;
  the bar for finishing the migration is the whole script passing clean.

## Refresh procedure after large refactors

After a large refactor, generate the ignored LAYER 1 output locally or download the CI
artifact for that exact commit, then refresh affected `## Graph context` sections from
`graphify query` output. Never stage `graphify-out/`.

## Related files

- `docs/templates/source-module.md` — v2.5 block-complete template; its 9 HARD RULES are
  what this script enforces.
- `sync-companion-code` / `context-pack` — exact-fidelity and context-index entry points.
- `.github/workflows/ci.yml` — runs this check and publishes LAYER 1 as a same-commit
  Graphify artifact; Graph context sections may also use an equivalent ignored local
  generation.
