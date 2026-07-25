# Keeplin agent guide

This file is the provider-neutral, canonical contract for every human or automated agent
working in either Keeplin repository. Read it completely before changing code or
documentation. Provider-specific files such as `CLAUDE.md` may add tool setup, but must not
repeat or override this contract.

## What Keeplin is and where work belongs

Keeplin is a pre-release, self-hostable notes system written in Rust. The repositories split
responsibility as follows:

- `jsunyermias/keeplin`
  - `keeplin-core`: domain models, storage backends, sync, collaboration client, shared wire
    and format contracts.
  - `keeplin-daemon`: gRPC and REST surfaces, configuration, authentication, metrics and
    local process lifecycle.
- `jsunyermias/keeplin-srv`
  - `crates/keeplin-srv`: the Axum/PostgreSQL multi-user server, collaborative WebSocket
    sessions, sync relay, accounts, devices and sharing.

Put shared types and constants in `keeplin-core`; the server consumes them from a pinned
revision. A change that crosses the repository boundary is one logical change and needs
coordinated PRs and contract tests.

## Commands

Run commands from the root of the repository being changed.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check-docs.sh
```

For `keeplin`, the focused suites are `cargo test -p keeplin-core` and
`cargo test -p keeplin-daemon`. For `keeplin-srv`, `cargo test --workspace` must run
against PostgreSQL and includes the `sqlx::test` integration tests described in its README.
When source changes affect the knowledge graph, install the CI-pinned version and also run:

```bash
pip install graphifyy==0.9.25
graphify update .
./scripts/check-graph.sh
```

Enable the repository's auto-refresh hook once with:

```bash
git config core.hooksPath .githooks
```

After `graphify update .`, commit the refreshed `graphify-out/` for code changes.

Never report a check as passing unless it ran successfully. Record unavailable checks and
their reason in the PR.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- `graphify update .` run and the refreshed `graphify-out/` committed (code changes only). CI (`scripts/check-graph.sh`) fails if the graph is stale; enable the auto-refresh hook once with `git config core.hooksPath .githooks`. Requires `pip install graphifyy==0.9.25`.

## Companion .md format

Companion .md format: docs/templates/source-module.md (v2.3.1, block-complete).
Read it fully before touching any companion .md. Its 9 HARD RULES are
contractual and scripts/check-docs.sh enforces them mechanically.

The nine rules, summarized without replacing the template, are:

1. Every leaf block is embedded complete and verbatim, including its `// md:` marker,
   attributes and full body.
2. Code fences never elide content. Split an oversized source block instead of shortening
   its companion fence.
3. A signature in Identification never substitutes for the complete Code fence.
4. Source block, `// md:` marker, companion section and coverage row have a strict 1:1:1:1
   correspondence.
5. A block implements one function, type, feature or inseparable small helper group and
   follows source order.
6. Containers document members as sub-blocks and do not duplicate their code; structural
   container scaffolding is exempt exactly as the template defines.
7. Before finishing, compare every fence with the source character-for-character and run
   the repository-wide docs check.
8. Dependencies name the exact symbols used and the behavioral contract each use expects.
9. Rust source contains no comments except `// md:` markers; explanation belongs in the
   companion.

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

## Persistence and wire invariants

These repository-specific rules are mandatory and are preserved verbatim from the previous
pull request templates.

### `keeplin`

- Proto changes (if any) are **additive** — new field numbers only, existing numbers never renumbered or reused.
- On-disk / on-wire format changes bump the relevant version (`FsBackend::FORMAT_VERSION`, `DbBackend::SCHEMA_VERSION`) with a migration step, and the change is documented in the companion.

### `keeplin-srv`

- New migrations are **forward-only** and idempotent (`ADD COLUMN IF NOT EXISTS`, `CREATE … IF NOT EXISTS`); existing migrations are never edited after being applied.
- New `NOT NULL` columns carry a `DEFAULT` so existing rows stay valid without a table rewrite.
- Every migration `.sql` has its companion `.md`.
- Any `SELECT` feeding a `sqlx::FromRow` struct includes all of that struct's columns (a missing column fails the row decode at runtime, not at compile time).

## Workflow and review independence

1. Start from an issue with observable acceptance criteria.
2. Create a dedicated branch; never work directly on `main`.
3. Open a draft PR and keep its scope limited to the issue.
4. Run the applicable checks and record evidence without conflating author claims with CI.
5. Have a different model family or a human independently review the objective and diff.
6. Resolve findings and conversations, obtain green required checks on the exact commit,
   then mark the PR ready for the maintainer to merge.

Start shared analysis with `docs/prompts/0.A-prompt-comun.md`. The default role split is
advisory: Claude documents and prepares issues, Kimi implements with
`docs/prompts/0.B-prompt-implementacion-issue.md`, and Codex reviews with
`docs/prompts/0.C-prompt-revision-seguridad.md`. The hard rule is that the implementer is
not the only reviewer. Independent review starts from the issue objective and the diff,
not from the author's defense as its sole source.

## ADR requirements

Security, persistence, migration, synchronization, protocol and other high-risk
architectural changes require an accepted ADR before implementation. The ADR records the
decision, alternatives, invariants, compatibility and migration impact, failure modes and
rollback or recovery plan. If the issue depends on an undecided ADR, stop at the decision
boundary rather than embedding an unreviewed architecture choice in code.

## Definition of done

A change is done only when its issue criteria are met; applicable code, companions, graph
and project docs agree; focused and repository checks pass or explicit blockers are
recorded; cross-repo surfaces and tests are coordinated; and an independent reviewer has
examined the objective and diff.

Do not call a change or deployment production-ready based only on happy-path tests. Claims
must cover failure behavior, security boundaries, persistence and recovery, compatibility,
operations and rollback evidence appropriate to the change.

## Protected `main`

Both repositories require changes through PRs, required status checks on the exact merge
candidate, resolved conversations, and protection against force-pushes and deletion. The
maintainer performs merges. See `CONTRIBUTING.md` for the contributor flow and repository
settings that enforce it.
