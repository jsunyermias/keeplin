## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).

## Companion .md format

Companion .md format: docs/templates/source-module.md (v2.3.1, block-complete).
Read it fully before touching any companion .md. Its 9 HARD RULES are
contractual and scripts/check-docs.sh enforces them mechanically.

## Documentation & Knowledge Consistency Policy

Documentation is part of the implementation, not a post-development task. A task is not
complete until the codebase, knowledge graph and documentation consistently describe the
same state of the project.

### Mandatory completion checks

Before marking any task as complete, perform the following verification steps:

1. Update every companion document corresponding to any modified source file so it
   accurately reflects the current implementation.
2. Regenerate Graphify whenever the changes affect architecture, dependencies, modules,
   classes, functions, relationships or any information represented in the knowledge
   graph.
3. Update every affected project document (for example: `ARCHITECTURE.md`, `README.md`,
   `SECURITY.md`, `CLAUDE.md`, ADRs or any other relevant documentation).
4. Verify that:
   - code and companions are consistent;
   - Graphify represents the current codebase;
   - documentation matches the implementation;
   - internal references and cross-references remain valid.
5. If any inconsistency is detected, resolve it before completing the task.

### Completion rule

Never consider a task finished while any known discrepancy exists between:

- source code;
- companion documentation;
- Graphify knowledge graph;
- project documentation.

The repository must always remain in a self-consistent state after every completed task.

## Cross-repo compatibility (keeplin ↔ keeplin-srv)

`keeplin` (client/daemon) and `keeplin-srv` (server) share a wire/format contract: the
collab protocol (`keeplin-core::collab::protocol`), `PROTOCOL_VERSION`
(`keeplin-core::compat`), the `Change` model, the format limits, and the encryption
envelope. Any change that touches a shared surface must keep both sides intercompatible —
it is not complete until that is guaranteed.

- `keeplin-core` is the single source of truth for shared wire/format types and constants;
  `keeplin-srv` imports them rather than redefining them.
- `keeplin-srv` pins `keeplin-core` to a concrete immutable reference (an exact `tag`/`rev`,
  never a branch or "latest") so the server can never silently drift from the client.
- A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both sides in lockstep.
- A change to a shared surface is not complete until a cross-repo compatibility test covers
  it: a round-trip of every protocol message and shared constant against `keeplin-core`'s
  real types.
