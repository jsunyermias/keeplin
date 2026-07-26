# `scripts/check-graph.sh` — build and validate the knowledge-graph artifact

## Purpose

Builds LAYER 1 of the navigation model for CI without requiring any generated file in git.
The script validates that `.graphifyignore` keeps the corpus focused, that the report has no
companion/template residue, and that two builds of the same tree have identical deterministic
structure. The workflow publishes the resulting ignored `graphify-out/` directory.

## How it works

1. Requires Graphify when `GRAPHIFY_REQUIRED=1`; otherwise a missing local install is a
   documented skip.
2. Runs `graphify update . --force` with the repository's `.graphifyignore` rules.
3. Validates the generated graph and report, then records a canonical snapshot that excludes
   only the derived `community` and `community_name` node fields.
4. Runs the same generation again and compares the deterministic nodes and edges with the
   snapshot. A difference fails the job.

## Corpus and signal checks

The generated graph must:

- contain non-empty nodes and edges, including at least one cross-file relationship;
- exclude `graphify-out/`, build/coverage/vendor trees, `docs/templates/` and companion
  Markdown;
- include Markdown only from the explicitly selected architecture, security and ADR corpus;
- expose at least three source-defined domain hubs and keep `Result`, `Vec` and `String`
  from naming any of the ten leading communities;
- produce a report with no `{{...}}` placeholders or companion `Coverage checklist` text.

These checks replace the old comparison with a committed `graph.json`. Freshness no longer
has meaning against repository state: CI generates the artifact from the exact checked-out
commit instead.

## Version pinning

CI installs `graphifyy==0.9.25`. Contributors who reproduce the artifact locally must use the
same pin. A version bump is a coordinated change to CI, `AGENTS.md`, this document and the PR
evidence.

## Local side effect

The script leaves a generated `graphify-out/` directory for local queries. The whole directory
is ignored; never stage or commit it. The former pre-commit auto-refresh hook was removed
because there is no versioned graph to refresh.

## Related files

- `.graphifyignore` — authoritative corpus exclusions and selected Markdown allow-list.
- `.github/workflows/ci.yml` — runs this script and publishes
  `knowledge-graph-<commit SHA>` with a 14-day retention.
- `scripts/check-docs.sh` — the sibling LAYER 2 companion gate.
