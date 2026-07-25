# Changelog

All notable changes to keeplin (keeplin-core + keeplin-daemon) are documented
here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

Client↔server compatibility is negotiated at runtime via keeplin-srv's
`GET /version` handshake (`protocol_version` + `capabilities`), so the crate
version and the wire protocol version move independently.

## [Unreleased]

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

- **`FsBackend` attachment layout** (`keeplin-core/src/storage/fs.rs`): attachments
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
- **Graphify integration**: committed knowledge graph
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
