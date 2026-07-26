# `scripts/context-pack` — reproducible companion context packs

## Complete source

```bash
# md:context-pack
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
if [[ $# -eq 0 ]]; then
  echo "usage: context-pack manifest [options] | context-pack TARGET [pack options]" >&2
  exit 2
fi
if [[ "$1" == "manifest" ]]; then
  shift
  exec python3 "$repo_root/scripts/companion_tool.py" manifest "$@"
fi
exec python3 "$repo_root/scripts/companion_tool.py" pack "$@"
```

## Purpose

Builds the generated context index and bounded ZIP inputs for models or reviewers. The
command is local and deterministic: it uses supported source markers (or the complete-file
SQL pairing) and authored companion metadata, never a network service or an LLM.

## Usage

- `./scripts/context-pack manifest` rewrites `docs/context-manifest.json`.
- `./scripts/context-pack manifest --check` verifies that the committed index is current.
- `./scripts/context-pack path/to/file.rs --list --profile understand` prints the exact
  planned file list, byte count and token estimate without creating a ZIP.
- `./scripts/context-pack path/to/file.rs --profile edit --output pack.zip` creates a ZIP.
- A unique Rust `// md:` or shell `# md:` marker path may replace the source path.

Profiles are deliberately narrow: `understand` includes only the target; `edit` adds only
dependencies explicitly inferred as indispensable contracts; `review` also adds direct
high-risk dependents; `cross-repo` adds the target's external-contract notes to the pack
metadata. `--max-files` (25) and `--max-bytes` (2,000,000) are hard default bounds.

Manifest schema 2 keeps invariant provenance per value: explicit `(EXTRACTED...)`
`**Invariants**` bullets remain `EXTRACTED`, while authored bullets and dependency
`expects:` clauses are `INFERRED` with a distinguishing `basis`. Ambiguous prose is not an
invariant. Cross-repo metadata contains only sorted stable identifiers declared under
`**Cross-repo contracts**`; incidental repo names and relationship prose are ignored.

ZIP entries are sorted, carry a fixed timestamp and fixed permissions, so identical inputs
produce identical bytes. The estimate is printed before the archive is written.

## Related files

- `companion_tool.py` — manifest extraction, risk classification and ZIP implementation.
- `../docs/context-manifest.json` — generated index with EXTRACTED/INFERRED provenance.
- `../docs/templates/source-module.md` — required companion sections.
