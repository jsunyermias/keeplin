# `scripts/check-graph.sh` — knowledge-graph freshness check

## Purpose

Enforces LAYER 1 of the navigation model the way `check-docs.sh` enforces LAYER 2:
it guarantees the committed `graphify-out/graph.json` still matches the code. Every
change to any indexed file must be accompanied by a `graphify update .` pass, and this
script is the CI gate that makes "the graph is always up to date" mechanical rather than
a matter of discipline.

## How it works

1. Snapshots the committed `graphify-out/graph.json`.
2. Runs `graphify update .` (AST-only, deterministic, **no LLM / no API cost**), which
   rebuilds the graph from the current tree.
3. Diffs the **code structure** of the snapshot against the rebuild and fails on any
   difference.

## What it compares — and what it ignores

Compared (the deterministic representation of the code):

- every node's `id`, `label`, `file_type`, `source_file`, `source_location`, `norm_label`
  and `_origin`;
- the full edge (`links`) set.

Ignored, on purpose:

- **`built_at_commit`** — records the commit the graph was built on, which is necessarily
  the *parent* of the commit that adds the refreshed graph, so it always differs in CI and
  is not a staleness signal.
- **`community` / `community_name`** — the Leiden clustering and its naming are a derived,
  navigation-only overlay. They re-shuffle on tiny input changes and may vary across
  environments or library versions; gating on them would produce spurious failures, so the
  check stays on the deterministic code structure instead. The committed graph still
  carries community data for navigation — the gate simply does not depend on it.

## graphify not installed

- **Default**: prints a hint and exits `0` (skips), so a contributor without graphify is
  never blocked from committing locally — CI remains the backstop.
- **`GRAPHIFY_REQUIRED=1`** (set by the CI job): a missing install is a hard failure
  instead of a silent skip, so a broken install can never quietly pass the gate.

## Version pinning

Community detection and extraction are deterministic **for a fixed graphify version and a
fixed input tree**. CI installs `graphifyy==0.9.25`; contributors must use the same pin so
their locally refreshed graph matches what CI rebuilds. Bumping the version is a
deliberate, coordinated change (update the CI pin, the hook hint and this doc together).

## Side effect

The script runs `graphify update .`, so it leaves the refreshed `graphify-out/` in the
working tree. On a stale tree that is the desired outcome: stage `graphify-out/` and commit
again.

## Related files

- `.github/workflows/ci.yml` — the `graph` job that runs this with `GRAPHIFY_REQUIRED=1`.
- `.githooks/pre-commit` — optional local hook that refreshes and stages the graph on every
  commit (the same freshness, applied automatically).
- `scripts/check-docs.sh` — the sibling gate for LAYER 2 (companion docs).
- `graphify-out/graph.json` — the artifact this check keeps honest.
