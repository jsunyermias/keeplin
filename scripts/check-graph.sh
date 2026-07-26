#!/usr/bin/env bash
# md:check-graph
# Build and validate the CI-owned Graphify artifact. The output is deliberately
# ignored by git: this check proves the configured corpus is useful and that two
# builds of the same tree have identical deterministic structure.
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

snapshot="$(mktemp)"
trap 'rm -f "$snapshot"' EXIT

graphify update . --force >/dev/null

python3 - "graphify-out/graph.json" "graphify-out/GRAPH_REPORT.md" "$snapshot" <<'PY'
import json
import pathlib
import re
import sys

graph_path = pathlib.Path(sys.argv[1])
report_path = pathlib.Path(sys.argv[2])
snapshot_path = pathlib.Path(sys.argv[3])

if not graph_path.is_file() or not report_path.is_file():
    raise SystemExit("check-graph: graphify did not produce graph.json and GRAPH_REPORT.md")

graph = json.loads(graph_path.read_text(encoding="utf-8"))
nodes = graph.get("nodes")
links = graph.get("links")
if not isinstance(nodes, list) or not nodes or not isinstance(links, list) or not links:
    raise SystemExit("check-graph: generated graph must contain non-empty nodes and links")

allowed_markdown = {
    "ARCHITECTURE.md",
    "SECURITY.md",
    "docs/adr/0000-template.md",
    "docs/adr/README.md",
}
banned_parts = {"graphify-out", "target", "build", "dist", "vendor", "coverage", "generated", "templates"}
sources = {}
violations = []
for node in nodes:
    node_id = node.get("id")
    source = node.get("source_file")
    if isinstance(node_id, str) and isinstance(source, str):
        source = source.replace("\\", "/").lstrip("./")
        sources[node_id] = source
        parts = set(pathlib.PurePosixPath(source).parts)
        if parts & banned_parts or source == "docs/context-manifest.json":
            violations.append(source)
        if source.endswith(".md") and source not in allowed_markdown and not source.startswith("docs/adr/"):
            violations.append(source)

if violations:
    sample = ", ".join(sorted(set(violations))[:10])
    raise SystemExit(f"check-graph: excluded corpus content entered the graph: {sample}")

cross_file_edges = 0
degree = {node.get("id"): 0 for node in nodes if isinstance(node.get("id"), str)}
for link in links:
    source_id = link.get("source")
    target_id = link.get("target")
    if source_id in degree:
        degree[source_id] += 1
    if target_id in degree:
        degree[target_id] += 1
    if sources.get(source_id) and sources.get(target_id) and sources[source_id] != sources[target_id]:
        cross_file_edges += 1

if cross_file_edges == 0:
    raise SystemExit("check-graph: generated graph lost all cross-file relationships")

labels = {node.get("id"): node.get("label") for node in nodes}
generic_labels = {"Result", "Vec", "String"}
real_entity_ids = [
    node_id
    for node_id in degree
    if sources.get(node_id, "").endswith(".rs")
    and labels.get(node_id) not in generic_labels
    and not str(labels.get(node_id, "")).endswith(".rs")
]
top_ids = sorted(real_entity_ids, key=lambda node_id: (-degree[node_id], node_id))[:10]
top_hubs = [labels.get(node_id) for node_id in top_ids]
if len(top_hubs) < 3:
    raise SystemExit("check-graph: generated graph has too few source-defined hubs")

report = report_path.read_text(encoding="utf-8")
for forbidden in ("{{", "Coverage checklist"):
    if forbidden in report:
        raise SystemExit(f"check-graph: generated report contains forbidden companion/template text: {forbidden}")
community_labels = re.findall(r'^### Community \d+ - ["“]([^"”]+)["”]$', report, flags=re.MULTILINE)
if generic_labels.intersection(community_labels[:10]):
    raise SystemExit("check-graph: a top-ten community is named after a generic type")

volatile_node_fields = {"community", "community_name"}
canonical_nodes = [
    json.dumps({key: value for key, value in node.items() if key not in volatile_node_fields}, sort_keys=True)
    for node in nodes
]
canonical_links = [json.dumps(link, sort_keys=True) for link in links]
snapshot_path.write_text(
    json.dumps({"nodes": sorted(canonical_nodes), "links": sorted(canonical_links)}, sort_keys=True),
    encoding="utf-8",
)

print(
    f"check-graph: generated {len(nodes)} nodes, {len(links)} edges "
    f"({cross_file_edges} cross-file); top hubs: {', '.join(str(hub) for hub in top_hubs)}"
)
PY

graphify update . --force >/dev/null

python3 - "graphify-out/graph.json" "$snapshot" <<'PY'
import json
import pathlib
import sys

graph = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
volatile_node_fields = {"community", "community_name"}
canonical = {
    "nodes": sorted(
        json.dumps({key: value for key, value in node.items() if key not in volatile_node_fields}, sort_keys=True)
        for node in graph["nodes"]
    ),
    "links": sorted(json.dumps(link, sort_keys=True) for link in graph["links"]),
}
expected = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
if canonical != expected:
    raise SystemExit("check-graph: two builds of the same tree produced different graph structure")

print("check-graph: corpus validation and same-tree reproducibility passed")
PY
