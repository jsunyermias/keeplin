<!--
  TEMPLATE: companion doc for a crate root (`lib.rs` or `main.rs`) that mostly declares and
  wires modules rather than carrying logic. Delete comments and unused sections before
  committing.
-->
# `{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}

## Purpose

<!-- What this crate is, and what this root file's job is (declare modules / wire the
     process). Keep it to a few sentences. -->
{{What the crate provides to the rest of the workspace, and the root file's role — declaring
public modules, or building the process and starting the servers.}}

## Module map

<!-- For a library root: the public surface. Mark what is exported. -->
| Module | Public | Description |
|--------|--------|-------------|
| `{{module}}` | {{yes/no}} | {{one line}} |

## Startup / wiring

<!-- For a binary root: the order things happen in at startup, and the decorator/middleware
     stack that gets built. A numbered list or a small diagram works well. Delete for a
     library root. -->
```
{{decorator or middleware stack, innermost -> outermost, or a numbered startup sequence}}
```

## Dependency graph (intra-crate)

<!-- Optional: how the modules depend on each other, so a reader knows what may import what. -->
```
{{module dependency sketch}}
```

## Design notes

- {{A convention this crate enforces (e.g. "no re-exports at the crate root, so import
  origins are explicit"; "one daemon per store, enforced before any I/O").}}

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes**

- `{{Entity}}` — {{what it is; the relationships the graph holds for it, labelled EXTRACTED/INFERRED}}

**Direct dependencies** (what this file uses; one line each on what it is and why it matters here)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Direct dependents** (who breaks if this file changes; one line each)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- {{invariant}}

## Related files

- `{{path}}` — {{one-line reason}}
- `ARCHITECTURE.md` — the one-page mental model this crate fits into
