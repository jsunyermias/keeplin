# Changelog

All notable changes to keeplin (keeplin-core + keeplin-daemon) are documented
here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

Client↔server compatibility is negotiated at runtime via keeplin-srv's
`GET /version` handshake (`protocol_version` + `capabilities`), so the crate
version and the wire protocol version move independently.

## [Unreleased]

### Trusted review-loop evaluation (keeplin ADR 0008)

- The authoritative evaluator now runs from a default-branch `workflow_run` workflow with no
  checkout or shell execution of pull-request content. Pull-request CI is explicitly read-only
  and proves its token receives 403 when attempting to patch a check run.
- Finding disposal now requires an independent MEMBER/OWNER/COLLABORATOR directive whose ID,
  author and body digest are reverified. Resolved findings additionally require a successful
  named check bound to the evaluated commit, configured workflow and App. Genesis and tombstones
  use the same authorization.
- The App comment journal detects edits and deletions with surviving descendants. Terminal
  truncation remains undetected and is pinned by `limitation_F002_terminal_truncation_undetected`;
  F-002 is dismissed by ADR 0008 and the option-C platform probes are tracked in
  `docs/review-loop-spike.md`. F-008 and F-013 are closed.

### Deterministic convergence for the review loop (keeplin ADR 0004)

- **The implementation↔review loop now terminates on a computed condition.** Previously the
  only mechanical gate was `.github/scripts/check-review-governance.js`, which never inspected
  findings: what stood for "the loop finished" was the pull-request checkbox `Blocking findings
  are resolved and conversations are closed`, an assertion by the agents inside the loop. No
  repository state held finding identity, round count or round-to-round comparison, so settled
  findings returned as new and a stalled loop was indistinguishable from a progressing one.
- **New `.github/scripts/check-review-loop.js`**, now driven by the default-branch trusted evaluator. A finding blocks only when *reified* — named as a
  test, property, contract assertion or `check-docs` check that fails; anything not reducible
  to a failing check is `advisory`, recorded but not blocking. Convergence is required checks
  green **and** zero open reified findings.
- **Findings are identified and durable.** The new `## Review ledger` section of
  `.github/pull_request_template.md` carries a stable ID and one state per finding (`open` /
  `resolved` / `dismissed` / `advisory`). A `dismissed` finding cites the priority decision or
  accepted ADR that settles it and does not reopen when re-raised, which is what stops a
  memoryless reviewer from restarting a settled loop.
- **The stagnation brake measures state, not a clock.** The loop-state hash is
  `sha256(normalized diff ‖ open reified finding IDs ‖ red check names)`. A repeated hash, or a
  blocking set that has not shrunk for `REVIEW_LOOP_STAGNATION_LIMIT` rounds (3), escalates to
  the maintainer naming the exact stuck item and demands an entry in the new
  `docs/review-stalls.md`. Iterating past a stall without that record fails CI.
- **The old checkbox became falsifiable rather than removed**: ticking "Blocking findings are
  resolved" while a reified finding is open now fails the check.
- **Independent review found decision-independent evaluator defects that are now fixed.**
  Convergence runs in its own `converge` job gated on
  `needs: [test, graph]`, because a step inside `Check, Test & Lint` asserted "required checks
  are green" before `cargo test`, Clippy, audit and the graph job had run. The job receives
  explicit `needs.test.result` and `needs.graph.result` values and requires `success`; skipped,
  neutral, absent and unknown block while optional checks do not. A body with
  no ledger section is round zero per the ADR's migration contract, not malformed. A stall
  record names every blocker as an explicit token in `Stuck on`, so `F-0010` cannot satisfy
  `F-001`. Canonical JSON frames hash fields and lists, preventing delimiter collisions.
  Markdown table parsing implements CommonMark backslash parity for pipes.
- **Historical limitation resolved by ADR 0008:** ADR 0004 read history from the editable PR
  body. ADR 0008 replaces that evaluator and honestly bounds the remaining terminal-truncation
  limitation; the older deliberately-red F-002 test is retired.
- **ADR 0005 is rejected and ADR 0006 is superseded by ADR 0008.** The implemented design keeps
  the default-branch evaluator and authenticated digest chain while withdrawing the false claim
  that terminal deletion is detectable.
- Independent review is untouched and conjunctive: convergence never ticks the review boxes,
  and a converged pull request with no independent reviewer is still unmergeable. `ci.yml`
  reads only its explicit required dependency results.

### Graphify graph moved to a CI artifact (keeplin#148)

- `graphify-out/` is no longer versioned. CI generates it with `graphifyy==0.9.25`,
  validates the focused corpus and same-tree reproducibility, then publishes
  `knowledge-graph-<commit SHA>` for 14 days.
- `.graphifyignore` excludes companions, templates and generated/build/vendor trees while
  retaining the selected architecture, security and ADR documents. The former pre-commit
  auto-refresh hook was removed because commits no longer carry generated graph files.

### Hard format limits, shared with keeplin-srv (#130)

- **New module `keeplin-core/src/format.rs`** — the single source of truth for three hard
  format limits, all exact powers of two: `MAX_LINE_BYTES` = 2¹² (4 096 UTF‑8 **bytes** per
  line), `MAX_LINES_PER_NOTE` = 2¹⁶ (65 536 live lines per note), `MAX_NOTES_PER_NOTEBOOK` =
  2²⁴ (16 777 216 live notes per notebook). It also owns the wire codes (`too_long`,
  `too_many_lines`, `notebook_full`). keeplin-srv imports these constants instead of declaring
  its own, so the two sides cannot drift; its previous values (`MAX_LINE_LEN = 10_000`,
  `MAX_LINES_PER_NOTE = 100_000`) are replaced. **Breaking, no migration**: content already
  over the new limits is refused by any path that revalidates it.
- **The client validates before writing** — `NoteLines::diff_body` now returns
  `Result<Vec<LineOp>, LimitViolation>` and rejects an over-limit body *before* mutating the
  mirror or emitting a single op; `CollabBackend::create_note`/`update_note` check the body
  before the local write. Previously the client knew nothing about the limits: an oversized
  edit looked saved locally, the server dropped it, and it never reached the user's other
  devices.
- **A server rejection is now repaired, not just logged** — `CollabServerMsg::Error` gained
  an optional `note_id` (additive; `PROTOCOL_VERSION` unchanged), and on a format-limit code
  the client drops that note's mirror and rejoins so the server's snapshot replaces the
  divergent local body.
- **New notes-per-notebook cap** — `NotebookSortProfile` gained `live_notes`, and
  `ordering::place_new_note` refuses a note once the destination notebook is full. Because
  `reconcile_notebook_move` routes through it, this covers both creating a note in a notebook
  and moving one into it.
- **New `StorageError::TooLarge`** — mapped to HTTP `413 Payload Too Large` in
  `keeplin-daemon/src/rest.rs` and gRPC `OUT_OF_RANGE` in `keeplin-daemon/src/server.rs`;
  both note surfaces validate the body up front, so the limits hold for a local-only daemon
  as well as in server mode.

### Filesystem format v8 — attachments in their note's folder (#127)

- **`FsBackend` attachment layout** (`keeplin-core/src/storage/fs/`): attachments
  leave the global `resources/{uuid}/` pool and live under their owning note as
  `notes/{note_id}/resources/{hash}.knrs` (original bytes, named by a BLAKE2s-256
  content hash) plus a `notes/{note_id}/resources/{id}.meta.ndjson` sidecar
  (`StoredResource` = the wire `Resource` + the fs-local `blob_hash`). Identical
  content in one note deduplicates onto a single blob; `purge_deleted_resources`
  reclaims a blob only when no live resource in that note still references its
  hash. The on-disk format version bumps to **8** (clean break — the old pool is
  no longer read). The shared wire `Resource` and the collab protocol are
  unchanged: the hash is storage-local and never crosses the wire, so keeplin ↔
  keeplin-srv stay intercompatible and `PROTOCOL_VERSION` is untouched. Adds a
  direct `blake2` dependency (already in the tree via `argon2`).

### 2026-07 production-readiness audit follow-up

- **Protocol handshake** (`keeplin-core/src/compat.rs`): the single place
  this repo defines server compatibility — `PROTOCOL_VERSION` +
  `compatible_with()` (exact match), mirrored by keeplin-srv's
  `src/http.rs`. `DbBackend::new` and `CollabBackend::start` now check
  `GET /version` at startup: compatible → negotiated protocol +
  capabilities logged (and the capability cache primed); incompatible →
  loud actionable failure naming which side to upgrade, no sync
  attempted; missing `/version` (old server) → warn and continue.
  `CollabBackend::start` now returns `Result`.
- **Out-of-band resource blobs actually land**: `create_resource` eagerly
  relays the blob-stripped `ResourceCreate` before uploading, and
  `upload_blob` checks the HTTP status with a short retry — previously
  the immediate `PUT` always lost the race against metadata
  materialisation and the blob was silently dropped (keeplin-srv 404s
  uploads for unknown resources).
- **Graphify integration (historical; graph storage superseded by keeplin#148)**: introduced a committed knowledge graph
  (`graphify-out/graph.json` + `GRAPH_REPORT.md`), mandatory
  `## Graph context` section in every companion `.md` (dependencies /
  dependents with inline summaries + restated invariants), CI-enforced by
  `scripts/check-docs.sh`, extended doc templates, and a README section
  on the two-layer (graph → companion docs) navigation model.

### Added
- iCalendar import reads **every** `VEVENT`/`VTODO` in a file, not just the
  first (`from_ics_all`, `import_todos`); the daemon import endpoints accept a
  whole calendar (#107).
- Optional `sync_interval_secs` daemon config: run a relay sync cycle on a
  cadence instead of only when a frontend polls (#111).
- The client negotiates keeplin-srv capabilities via `GET /version` and skips
  features the server does not advertise (keeplin#114).
- Collaborative note discovery pages through `GET /api/notes` with
  `?limit=&cursor=`, following the server's `X-Next-Cursor` header, so a large
  account is not fetched in a single unbounded response. Back-compatible with a
  server that predates pagination (keeplin-srv#29).

### Changed
- Server-backed note/notebook history in server mode: `DbBackend` fetches the
  server's history (every device's changes) with a local-journal fallback, and
  latches a `404`/absent capability to avoid wasted round-trips (#100 follow-up, #113).
- `CollabBackend` checks the HTTP status of the note POST/PATCH mirror and logs
  a server rejection instead of treating it as delivered (#112).
- The alias index no longer holds every Inbox note id; the uuid→Inbox check is a
  backend read at write time, so the index is bounded by the alias count (#106).
- Contact/event by-UID operations resolve from resource **metadata** (the
  `<uid>.vcf`/`.ics` file name) instead of scanning every payload (#105).

### Documentation
- `SECURITY.md` documents that collaborative mode stores note title/body in
  cleartext on the server, and how to avoid it (#110).

## [0.1.0]

- Initial client: `FsBackend` (Syncthing) and `DbBackend` (server relay +
  collaborative channel) storage, at-rest encryption, notebooks/tags/resources,
  links & aliases, ordering/pinning/starring, history & revert, vCard/iCalendar
  interop, and the gRPC + REST/WebSocket daemon surfaces.
