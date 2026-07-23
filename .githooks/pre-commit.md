# `.githooks/pre-commit` — auto-refresh the knowledge graph on commit

## Purpose

An optional local git hook that keeps `graphify-out/` current automatically, so a commit
never lands with a stale graph. It applies the same freshness that CI enforces via
`scripts/check-graph.sh`, but at commit time instead of after a failed CI run.

## Enabling

Once per clone:

```sh
git config core.hooksPath .githooks
```

This points git at the tracked `.githooks/` directory instead of `.git/hooks/`, so the
hook travels with the repo and needs no per-file install.

## Behaviour

On every commit:

1. Runs `graphify update .` (AST-only, no LLM / no API cost).
2. `git add`s the refreshed `graphify-out/graph.json`, `GRAPH_REPORT.md` and
   `.graphify_labels.json` so they are part of the commit.

Escape hatches:

- **`SKIP_GRAPHIFY=1`** — bypass the hook for a single commit
  (`SKIP_GRAPHIFY=1 git commit …`).
- **graphify not installed** — the hook prints a hint and lets the commit through; CI is
  the backstop.

## Version

Install the pinned graphify so the locally refreshed graph matches what CI rebuilds:
`pip install graphifyy==0.9.25`.

## Related files

- `scripts/check-graph.sh` — the CI gate this hook keeps you ahead of.
- `.github/workflows/ci.yml` — the `graph` job that fails on a stale graph.
- `graphify-out/graph.json` — the artifact this hook refreshes and stages.
