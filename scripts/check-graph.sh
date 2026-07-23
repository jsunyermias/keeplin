#!/usr/bin/env bash
# Knowledge-graph freshness check (CI-enforced). Re-runs `graphify update .`
# (AST-only, deterministic, no LLM/API cost) and fails if the committed
# graphify-out/graph.json no longer matches the rebuild — i.e. someone changed
# code without refreshing the graph.
#
# What is compared: the code structure only — every node's identity and location
# (id, label, file_type, source_file, source_location, norm_label, _origin) and
# the full edge set. What is deliberately ignored:
#   - `built_at_commit`: records the commit the graph was built on, necessarily
#     the parent of the commit that adds the refreshed graph, so it always
#     differs and is not a staleness signal.
#   - `community` / `community_name`: Leiden clustering + naming is a derived,
#     navigation-only overlay that re-shuffles on tiny input changes and could
#     vary across environments; gating on it would cause spurious failures, so
#     the check stays on the deterministic code structure instead.
#
# graphify not installed:
#   - default: warn and skip (exit 0) so local commits are never blocked for
#     contributors without graphify; CI still enforces.
#   - GRAPHIFY_REQUIRED=1 (set in CI): hard-fail instead of skipping, so a
#     broken install can never silently pass the gate.
#
# Side effect: this runs `graphify update .`, so it leaves the refreshed
# graphify-out/ in the working tree. On a stale tree that is exactly what you
# want — stage it and commit again.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v graphify >/dev/null 2>&1; then
  if [[ "${GRAPHIFY_REQUIRED:-0}" == "1" ]]; then
    echo "check-graph: graphify is required but not installed (pip install graphifyy==0.9.25)" >&2
    exit 1
  fi
  echo "check-graph: graphify not installed; skipping (pip install graphifyy==0.9.25 to enable, or set GRAPHIFY_REQUIRED=1 to enforce)" >&2
  exit 0
fi

before="$(mktemp)"
trap 'rm -f "$before"' EXIT
cp graphify-out/graph.json "$before"

graphify update . >/dev/null

python3 - "graphify-out/graph.json" "$before" <<'PY'
import json, sys

VOLATILE_NODE_FIELDS = ("community", "community_name")


def structure(path):
    g = json.load(open(path))
    nodes = []
    for n in g["nodes"]:
        n = {k: v for k, v in n.items() if k not in VOLATILE_NODE_FIELDS}
        nodes.append(json.dumps(n, sort_keys=True))
    links = [json.dumps(e, sort_keys=True) for e in g["links"]]
    return sorted(nodes), sorted(links)


rebuilt = structure(sys.argv[1])
committed = structure(sys.argv[2])

if rebuilt != committed:
    print(
        "::error::graphify-out/ is stale — the AST graph structure changed but the "
        "committed graph.json did not. This check already ran `graphify update .`; "
        "stage the refreshed graphify-out/ and commit again.",
        file=sys.stderr,
    )
    sys.exit(1)

print("graphify-out/ is up to date")
PY
