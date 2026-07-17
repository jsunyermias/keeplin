#!/usr/bin/env bash
# Contractual-docs check (CI-enforced):
#   1. every .rs file has a companion .md (same path, .rs -> .md), and
#   2. every such companion contains a `## Graph context` section.
#
# The companion .md system is LAYER 2 of the navigation model (see README,
# "Navigating this repo"); the Graphify graph (graphify-out/graph.json) is
# LAYER 1. After large refactors, refresh LAYER 1 with `graphify update .`
# and the affected `## Graph context` sections with it.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
while IFS= read -r -d '' rs; do
  md="${rs%.rs}.md"
  if [[ ! -f "$md" ]]; then
    echo "MISSING companion doc: $md (for $rs)"
    fail=1
  elif ! grep -q '^## Graph context' "$md"; then
    echo "MISSING '## Graph context' section in $md"
    fail=1
  fi
done < <(find . \
  -path ./target -prune -o \
  -path ./graphify-out -prune -o \
  -path ./.git -prune -o \
  -name '*.rs' -print0)

if [[ $fail -ne 0 ]]; then
  echo
  echo "Every .rs needs a companion .md with a '## Graph context' section."
  echo "Template: docs/templates/source-module.md (or test-file.md)."
  exit 1
fi
echo "docs check OK: every .rs has a companion .md with a Graph context section"
