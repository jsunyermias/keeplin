# `storage/db.rs` — DbBackend (LibSQL + WebSocket storage)

Self-contained companion for `keeplin-core/src/storage/db.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use std::sync::Arc;
// … async_trait, base64, chrono, futures_util, tokio::sync::{Mutex, RwLock},
// tokio::time, tokio_tungstenite, uuid; crate error/links/models types;
// super::{backend::DEFAULT_HISTORY_LIMIT, note_log::{resolve, VersionVector, Winner},
//         EntityVersion, the repository traits, SortableRfc3339}
```

**What it does** — The LibSQL-backed `StorageBackend` with WebSocket
synchronisation. All data lives in a local SQLite-compatible database; every
mutation also appends a row to the `entity_changes` journal so
`get_changes_since` can enumerate ordered mutations after any instant. Binary
resource payloads are stored as BLOBs and embedded in the journal as Base64
(`_data_b64`) so peers can reconstruct resources from the journal alone.
Conflict resolution is **version vectors for every entity**
(`note_log::resolve` over the stored vs incoming `(vv, updated_at, last_writer)`)
— the same decision procedure `FsBackend` uses via log-based `merge`; only the
storage shape differs (see `SECURITY.md`).

**Dependencies** — `libsql`, `tokio_tungstenite`, `reqwest` (server history +
`/version`), `base64`, `serde_json`; `note_log`, the trait family,
`SortableRfc3339` (every stored timestamp uses the fixed nine-digit RFC 3339
shape so lexicographic = chronological).

**Used by** — `keeplin-daemon/src/main.rs` (`storage = "database"` mode),
`migrate.rs`, the DbBackend integration tests (`tests/db_backend.rs`,
`tests/ws_sync.rs`, `tests/sync.rs`).

**Repeated context** — the storage conventions restated: soft-delete-always,
idempotent `apply_change` (equal vectors → `Winner::Local` → no-op), cursor
pagination (`"<ts>|<uuid>"` keyset), and the `(timestamp, device_id)` LWW
tiebreak.

---

## WsStream

**Identification** —
`type WsStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>;`
marker `// md:WsStream`.

**What it does** — The WebSocket stream over plain TCP or TLS —
`MaybeTlsStream` handles both, so `ws://` and `wss://` need no type change.

**Dependencies** — `tokio_tungstenite`. **Used by** — the `ws` field,
`connect_ws`, `ensure_ws`. **Repeated context** — none.

---

## DbBackend

**Identification** — `pub struct DbBackend`; marker `// md:DbBackend`.

**What it does** — The backend's state: `conn` (the open LibSQL connection),
`server_url` (`ws://`/`wss://`; empty = offline mode, no WebSocket),
`auth_token` (bearer token sent as the first WebSocket message; kept so
`ensure_ws` can re-authenticate on reconnect), `ws: Arc<Mutex<Option<WsStream>>>`
(`None` = not configured or lost), `device_id` (permanent installation UUID from
the `device` table; sent in every change batch), `http` (REST client for
history/`/version`), `history_unsupported: AtomicBool` (latched once the server
answers a history request with 404 — subsequent reads skip the wasted round-trip,
issue #113; a network error does **not** latch it),
`server_capabilities: Mutex<CapabilityCache>` (cached `GET /version` outcome,
keeplin#114), and `lock: Arc<RwLock<()>>` — the connection guard: writers take
the exclusive side for whole `BEGIN IMMEDIATE … COMMIT` spans (and bare writes),
readers the shared side, guaranteeing on the single shared connection that two
`BEGIN IMMEDIATE`s never overlap, a bare write never lands inside another task's
open transaction, and a query never observes uncommitted rows. SQLite allows
only one writer anyway, so the exclusive side costs no real throughput.

If the WebSocket fails, the local database keeps working and changes accumulate
in `entity_changes`; a `send_changes` that cannot deliver **errors** so the sync
cycle aborts without advancing the watermark — undelivered changes are re-sent
next cycle, never silently skipped.

**Dependencies** — `libsql`, `tokio` sync, `reqwest`.

**Used by** — everything below.

**Repeated context** — none.

---

## impl DbBackend

**Identification** — the first inherent impl; marker `// md:impl DbBackend`.
Constructor, migrations, journal/WS/row/versioning helpers.

### fn new

**Identification** —
`pub async fn new(db_path, server_url, auth_token) -> Result<Self, StorageError>`;
marker `// md:impl DbBackend > fn new`.

**What it does** — Opens (or creates) the database, runs `run_migrations`, loads
or creates the device id, builds a 10 s-timeout HTTP client, then — when a
server is configured — runs the **`GET /version` protocol handshake** before any
sync: `Compatible` logs and primes the capability cache; `Incompatible` fails
construction loudly with the actionable `incompatible_message`; `Unavailable`
warns and continues (older keeplin-srv / bare test relay). Finally attempts the
WebSocket connection — a failure is a **non-fatal warning** (offline mode), never
a constructor error.

**Dependencies** — `libsql::Builder`, `run_migrations`,
`get_or_create_device_id`, `compat::negotiate`, `connect_ws`, `http_base_of`.

**Used by** — `main.rs::build_storage`; `migrate` subcommand; tests.

**Repeated context** — none.

### fn run_migrations

**Identification** — `async fn run_migrations(conn) -> Result<(), StorageError>`;
marker `// md:impl DbBackend > fn run_migrations`.

**What it does** — Brings the schema up to `SCHEMA_VERSION`, recording progress
in `PRAGMA user_version` so each step runs exactly once across restarts. An
up-to-date database does no schema work; each outstanding step runs in its own
`BEGIN IMMEDIATE` transaction with the version stamp set **inside** it (SQLite
DDL is transactional; the stamp rolls back with a failed step, so a crash
mid-migration retries cleanly). A database whose `user_version` is **newer**
than this build is rejected (`InvalidState`) so a downgrade cannot corrupt a
schema it doesn't understand. `PRAGMA user_version = {n}` takes no bound
parameters; `n` is our own const, never caller input.

**Dependencies** — `schema_version`, `apply_migration`.

**Used by** — `new`. **Repeated context** — none.

### fn schema_version

**Identification** — marker `// md:impl DbBackend > fn schema_version`.

**What it does** — Reads `PRAGMA user_version` (`0` for a pre-framework or
never-stamped database).

**Used by** — `run_migrations`; migration tests.

### fn apply_migration

**Identification** — marker `// md:impl DbBackend > fn apply_migration`.

**What it does** — Dispatches the step that advances **to** `version`: 1 →
`migrate_v1_baseline`, 2 → `migrate_v2_ordering`, anything else →
`InvalidState`. The caller wraps it in a transaction and bumps the stamp.

**Used by** — `run_migrations`.

### fn migrate_v1_baseline

**Identification** — marker `// md:impl DbBackend > fn migrate_v1_baseline`.

**What it does** — The baseline schema in one step: tables `notes`, `notebooks`,
`tags`, `note_tags` (versioned associations), `note_links` (projection of each
note's resolved outgoing links so backlinks are an indexed lookup; target UUIDs
are plaintext so the index works under at-rest encryption), `resources`
(metadata + BLOB), `sync_state`, `device`, `entity_changes` (append-only
journal; auto-increment `id` breaks `changed_at` ties; `data` holds full entity
JSON for create/update, `NULL` for deletes, and `_data_b64` for resource
payloads) plus the supporting indexes. All `IF NOT EXISTS`, followed by
`add_column_if_missing` guards for every column added since v0
(alias/bookmarks/links, `vv`/`last_writer` on all entities, versioned
`note_tags` columns, resource soft-delete columns) — so a pre-framework
database, which already has them, is carried onto the ladder at `1` unchanged.
**Deliberately no `UNIQUE` index for aliases**: under at-rest encryption the
stored alias is per-write ciphertext (fresh nonce, never compares equal), and a
hard constraint would make `apply_change` error on a sync-introduced duplicate
instead of tolerating it — `LinkingBackend` enforces uniqueness on plaintext at
the application layer.

**Used by** — `apply_migration`.

### fn add_column_if_missing

**Identification** — marker `// md:impl DbBackend > fn add_column_if_missing`.

**What it does** — `ALTER TABLE … ADD COLUMN …`, treating "duplicate column
name" as success.

**Used by** — the migration steps.

### fn get_or_create_device_id

**Identification** — marker `// md:impl DbBackend > fn get_or_create_device_id`.

**What it does** — Reads the single `device` row, or inserts a fresh UUID v4 on
first startup. Included in every change batch so the relay keeps a per-device
delivery cursor and never echoes a device's own changes back.

**Used by** — `new`.

### fn record_change

**Identification** — marker `// md:impl DbBackend > fn record_change`.

**What it does** — Inserts one `entity_changes` row (`entity_type` ∈ note/
notebook/tag/note_tag/resource; `operation` ∈ create/update/delete/add/remove;
`changed_at = now()` in sortable form; `data` = full entity JSON or `None`).
Called by every mutating method inside the same transaction as the primary
write, so the pair commits or rolls back together.

**Used by** — every write path below.

### fn refresh_note_links

**Identification** — marker `// md:impl DbBackend > fn refresh_note_links`.

**What it does** — Rebuilds the `note_links` projection for one note: delete
its rows, insert one per distinct resolved `target_note_id`. Called on every
note write (local and applied sync) so backlinks stay indexed.

**Used by** — `create_note`, `update_note`, `apply_change`.

### fn connect_ws

**Identification** — marker `// md:impl DbBackend > fn connect_ws`.

**What it does** — Opens the WebSocket and performs the application-level
handshake: first message `{"type":"auth","token":…,"device_id":…}`. The server
validates the token or closes; a later closure is detected by
`send_changes`/`receive_changes`, which clear the slot and trigger a reconnect.
The `device_id` lets the relay keep a per-device delivery cursor and replay
missed batches on reconnect (keeplin-srv's durable journal); older relays
ignore it. **Security note**: the token travels in the socket — use `wss://` in
production.

**Used by** — `new`, `ensure_ws`.

### fn row_to_note

**Identification** — marker `// md:impl DbBackend > fn row_to_note`.

**What it does** — Maps an 18-column `notes` row (the fixed SELECT order used
everywhere) to a `Note`: NULL `notebook_id` → nil UUID (Inbox), JSON columns
via the lenient `json_to_*` parsers, `sort_key` clamped non-negative.

**Used by** — every note read/list.

### fn parse_uuid

**Identification** — marker `// md:impl DbBackend > fn parse_uuid`.

**What it does** — `String → Uuid`, mapping failure to `InvalidState`
(corrupted row, server bug — not a caller error).

### fn parse_required_dt

**Identification** — marker `// md:impl DbBackend > fn parse_required_dt`.

**What it does** — `String → DateTime<Utc>`, failure → `InvalidState`.

### fn parse_optional_dt

**Identification** — marker `// md:impl DbBackend > fn parse_optional_dt`.

**What it does** — `Option<String> → Option<DateTime<Utc>>`, failure →
`InvalidState`.

### fn row_to_notebook

**Identification** — marker `// md:impl DbBackend > fn row_to_notebook`.

**What it does** — Maps the 8-column `notebooks` row shape.

### fn row_to_tag

**Identification** — marker `// md:impl DbBackend > fn row_to_tag`.

**What it does** — Maps the 7-column `tags` row shape.

### fn row_to_resource

**Identification** — marker `// md:impl DbBackend > fn row_to_resource`.

**What it does** — Maps the 9-column metadata row shape (no `data` BLOB).

### fn row_to_change

**Identification** — marker `// md:impl DbBackend > fn row_to_change`.

**What it does** — Converts an `entity_changes` row into a typed `Change`:
create/update deserialise the full JSON; deletes reconstruct
`(deleted_at, vv, last_writer)` via `tombstone_from_data` (with `changed_at` as
fallback); note_tag add/remove via `assoc_from_data`; resource creates decode
`_data_b64` into `ResourceCreate.data`. Returns `None` for unknown
`(entity_type, operation)` pairs (a future build's rows) — callers log and skip
without aborting the sync.

**Used by** — `get_changes_since`.

### fn begin

**Identification** — marker `// md:impl DbBackend > fn begin`.

**What it does** — `BEGIN IMMEDIATE` (write lock up front so all writes in the
span commit or roll back atomically — the primary write and the journal row can
never diverge).

### fn commit

**Identification** — marker `// md:impl DbBackend > fn commit`.

**What it does** — `COMMIT`.

### fn rollback

**Identification** — marker `// md:impl DbBackend > fn rollback`.

**What it does** — `ROLLBACK`, errors swallowed (a rollback failure means no
transaction was active — already clean).

### fn ensure_ws

**Identification** — marker `// md:impl DbBackend > fn ensure_ws`.

**What it does** — Reconnects when the slot is empty and a URL is configured;
on failure the slot stays `None` and the caller skips the network operation
(changes accumulate locally).

**Used by** — `send_changes`, `receive_changes`.

### fn migrate_v2_ordering

**Identification** — marker `// md:impl DbBackend > fn migrate_v2_ordering`.

**What it does** — Migration v2: `is_pinned`/`is_starred`/`sort_key` columns
(defaults keep old rows valid — `sort_key 0` is the never-positioned sentinel);
existing `NULL notebook_id` rows are moved to the Inbox (nil UUID) so queries
never see NULL again; and the `(notebook_id, sort_key, id)` index behind
`list_notes_in_notebook`.

**Used by** — `apply_migration`.

### fn current_meta

**Identification** — marker `// md:impl DbBackend > fn current_meta`.

**What it does** — Reads `(vv, updated_at, last_writer)` of a row in
`notes`/`notebooks`/`tags` (hard-coded table literals — interpolation is safe),
or `None` when absent. Feeds `resolve`.

**Used by** — `incoming_wins`, `next_local_vv`.

### fn incoming_wins

**Identification** — marker `// md:impl DbBackend > fn incoming_wins`.

**What it does** — Whether an incoming remote write replaces the local row:
`true` with no local row, else `resolve(local, incoming) == Winner::Incoming`.
Replaces the old bare-`updated_at` LWW so concurrent edits converge
deterministically.

**Used by** — `apply_change` (notes/notebooks/tags).

### fn next_local_vv

**Identification** — marker `// md:impl DbBackend > fn next_local_vv`.

**What it does** — The vector for a **local** write: current stored vector (or
empty) with this device's component incremented; the caller stamps the entity
and sets `last_writer = device_id`.

**Used by** — every local create/update/delete.

### fn row_is_live

**Identification** — marker `// md:impl DbBackend > fn row_is_live`.

**What it does** — Whether a row exists with `deleted_at IS NULL`
(`notes`/`tags` literals only). Used to refuse dangling associations.

**Used by** — `add_note_tag`.

### fn assoc_meta

**Identification** — marker `// md:impl DbBackend > fn assoc_meta`.

**What it does** — Version metadata of a note↔tag association; a pre-version
row (NULL `updated_at`) is reported at the epoch so any real write dominates.

**Used by** — `next_assoc_vv`, `assoc_incoming_wins`.

### fn next_assoc_vv

**Identification** — marker `// md:impl DbBackend > fn next_assoc_vv`.

**What it does** — Local association write vector (current + increment).

**Used by** — `add_note_tag`, `remove_note_tag`.

### fn assoc_incoming_wins

**Identification** — marker `// md:impl DbBackend > fn assoc_incoming_wins`.

**What it does** — `resolve` for association writes; `true` when the pair has
no row.

**Used by** — `apply_change` (NoteTagAdd/Remove).

### fn upsert_assoc

**Identification** — marker `// md:impl DbBackend > fn upsert_assoc`.

**What it does** — `INSERT OR REPLACE` of the association's versioned state:
`deleted_at = NULL` for an add (present), `Some(ts)` for a remove (tombstone).

**Used by** — `add_note_tag`, `remove_note_tag`, `apply_change`.

### fn resource_meta

**Identification** — marker `// md:impl DbBackend > fn resource_meta`.

**What it does** — Resource version metadata; resources have no `updated_at`,
so the tiebreak timestamp is `deleted_at` when tombstoned else `created_at`.

**Used by** — `next_resource_vv`, `resource_incoming_wins`.

### fn next_resource_vv

**Identification** — marker `// md:impl DbBackend > fn next_resource_vv`.

**What it does** — Local resource write vector (current + increment).

**Used by** — `create_resource`, `delete_resource`.

### fn resource_incoming_wins

**Identification** — marker `// md:impl DbBackend > fn resource_incoming_wins`.

**What it does** — `resolve` for resource changes; `true` with no local row.

**Used by** — `apply_change` (ResourceCreate/Delete).

---

## fn parse_cursor

**Identification** — `fn parse_cursor(token: Option<&str>) -> (String, String)`;
marker `// md:fn parse_cursor`.

**What it does** — Splits a `"<created_at>|<uuid>"` cursor into its parts;
absent/empty/malformed → `("", "")`, which makes the keyset SQL condition
`?1 = ''` match all rows (no offset).

**Used by** — every list method. **Repeated context** — none.

---

## fn build_page

**Identification** —
`fn build_page<T, F>(rows: Vec<T>, limit: usize, token_fn: F) -> (Vec<T>, Option<String>)`;
marker `// md:fn build_page`.

**What it does** — Turns a `LIMIT limit + 1` fetch into `(page, next_token)`:
more than `limit` rows ⇒ truncate and build the token from the page's last item;
otherwise no token.

**Used by** — every list method. **Repeated context** — none.

---

## fn bookmarks_to_json

**Identification** — marker `// md:fn bookmarks_to_json`.

**What it does** — Serialises `notes.bookmarks` (`"[]"` fallback — a `Vec` of
small structs cannot fail in practice).

---

## fn links_to_json

**Identification** — marker `// md:fn links_to_json`.

**What it does** — Serialises `notes.links` (`"[]"` fallback).

---

## fn json_to_bookmarks

**Identification** — marker `// md:fn json_to_bookmarks`.

**What it does** — Parses the bookmarks column; malformed → empty list rather
than failing the read.

---

## fn json_to_links

**Identification** — marker `// md:fn json_to_links`.

**What it does** — Parses the links column; malformed → empty list.

---

## fn vv_to_json

**Identification** — marker `// md:fn vv_to_json`.

**What it does** — Serialises a version vector (`"{}"` fallback).

---

## fn json_to_vv

**Identification** — marker `// md:fn json_to_vv`.

**What it does** — Parses a `vv` column; malformed → empty vector (behaves as
an uninformed write).

---

## fn tombstone_data

**Identification** — marker `// md:fn tombstone_data`.

**What it does** — Builds the journal `data` JSON for a delete:
`deleted_at` + the deleting write's `vv`/`last_writer`, so `row_to_change`
reconstructs a delete `Change` carrying everything `resolve` needs on the
receiving peer.

**Used by** — every delete path.

---

## fn assoc_data

**Identification** — marker `// md:fn assoc_data`.

**What it does** — Journal `data` JSON for a note↔tag add/remove: `tag_id` +
version metadata.

**Used by** — `add_note_tag`, `remove_note_tag`.

---

## fn assoc_from_data

**Identification** — marker `// md:fn assoc_from_data`.

**What it does** — Reconstructs `(updated_at, vv, last_writer)` from a journal
value, falling back to `changed_at` and empty vv/writer for pre-version
records.

**Used by** — `row_to_change`.

---

## fn tombstone_from_data

**Identification** — marker `// md:fn tombstone_from_data`.

**What it does** — Reconstructs `(deleted_at, vv, last_writer)` from a journal
value, same fallbacks.

**Used by** — `row_to_change`.

---

## impl NoteRepository for DbBackend

**Identification** — marker `// md:impl NoteRepository for DbBackend`; each
method carries `// md:impl NoteRepository for DbBackend > fn <name>`.

**What it does** — the note surface. Common write pattern: exclusive lock →
`begin` → stamp `vv = next_local_vv` + `last_writer = device_id` → primary
write (+ `refresh_note_links` for notes) → `record_change` with the full
snapshot → `commit` (or `rollback` on any error).

### fn create_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn create_note`.

**What it does** — Plain `INSERT` (an existing id errors — fresh-destination
contract), links projection refresh, `"create"` journal row with the full
snapshot; returns the stamped note.

### fn read_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn read_note`.

**What it does** — Single-row SELECT (18 columns); `NotFound` when absent.
Note: tombstoned rows **are** returned (needed for resolution and revival);
user-facing layers re-check `deleted_at`.

### fn update_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn update_note`.

**What it does** — `UPDATE … WHERE id` (0 rows → `NotFound`), links refresh,
`"update"` journal row.

### fn delete_note

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn delete_note`.

**What it does** — Soft delete: `deleted_at = updated_at = now`, bumped vv,
`"delete"` journal row with `tombstone_data`.

### fn list_notes

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_notes`.

**What it does** — Live notes in `(created_at, id)` order with the
`"<ts>|<id>"` keyset cursor, `LIMIT limit + 1` + `build_page`.

### fn note_backlinks

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn note_backlinks`.

**What it does** — The indexed override of the trait default: `note_links`
joined back to live notes (`idx_note_links_target` makes the WHERE an index
seek), keyset cursor on `(created_at, id)`.

### fn list_notes_in_notebook

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_notes_in_notebook`.

**What it does** — One notebook's live notes ordered by the **effective** sort
key — the SQL `CASE WHEN sort_key = 0 THEN 1000 …` mirrors
`Note::effective_sort_key`, and the cursor carries the effective key compared
numerically (`CAST`).

### fn list_starred_notes

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn list_starred_notes`.

**What it does** — `is_starred = 1` live notes, `(created_at, id)` keyset.

### fn notebook_sort_profile

**Identification** — marker
`// md:impl NoteRepository for DbBackend > fn notebook_sort_profile`.

**What it does** — Keys-only scan (`idx_notes_notebook_sort`) mapped through
the 0→`DEFAULT_SORT_KEY` sentinel into
`NotebookSortProfile::from_effective_keys`.

---

## impl NotebookRepository for DbBackend

**Identification** — marker `// md:impl NotebookRepository for DbBackend`;
per-method markers `> fn <name>`.

**What it does** — the notebook CRUD, same transactional write pattern.

### fn create_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn create_notebook`.

**What it does** — Stamped `INSERT` + `"create"` journal row.

### fn read_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn read_notebook`.

**What it does** — Single-row SELECT; `NotFound` when absent.

### fn update_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn update_notebook`.

**What it does** — Stamped `UPDATE` (0 rows → `NotFound`) + `"update"` row.

### fn delete_notebook

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn delete_notebook`.

**What it does** — Soft delete + tombstone journal row.

### fn list_notebooks

**Identification** — marker
`// md:impl NotebookRepository for DbBackend > fn list_notebooks`.

**What it does** — Live notebooks, `(created_at, id)` keyset.

---

## impl TagRepository for DbBackend

**Identification** — marker `// md:impl TagRepository for DbBackend`;
per-method markers `> fn <name>`.

**What it does** — tags + versioned associations.

### fn create_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn create_tag`.

**What it does** — Stamped `INSERT` + journal row.

### fn read_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn read_tag`.

**What it does** — Single-row SELECT; `NotFound` when absent.

### fn update_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn update_tag`.

**What it does** — Stamped `UPDATE` (0 rows → `NotFound`) + journal row.

### fn delete_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn delete_tag`.

**What it does** — Soft delete + tombstone journal row.

### fn list_tags

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn list_tags`.

**What it does** — Live tags, `(created_at, id)` keyset.

### fn add_note_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn add_note_tag`.

**What it does** — Verifies **both ends are live** (`row_is_live`; `NotFound`
otherwise — the API must not create dangling associations; `apply_change`
deliberately skips this because sync delivery order is not guaranteed), then
`upsert_assoc` with `deleted_at = NULL` (the present state, versioned so a
concurrent add-vs-remove converges) + an `"add"` journal row. Idempotent.

### fn remove_note_tag

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn remove_note_tag`.

**What it does** — `upsert_assoc` with a tombstone (kept so it can beat a
concurrent add) + a `"remove"` journal row. Idempotent.

### fn list_note_tags

**Identification** — marker
`// md:impl TagRepository for DbBackend > fn list_note_tags`.

**What it does** — Tags joined through live (`nt.deleted_at IS NULL`)
associations, `(created_at, id)` keyset.

---

## impl ResourceRepository for DbBackend

**Identification** — marker `// md:impl ResourceRepository for DbBackend`;
per-method markers `> fn <name>`.

**What it does** — resources with BLOB payloads.

### fn create_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn create_resource`.

**What it does** — Stamped `INSERT` storing the BLOB, plus a `"create"` journal
row whose JSON carries `_data_b64` (the Base64 payload) so peers receiving the
change via the relay reconstruct the full resource without a separate download.

### fn read_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn read_resource`.

**What it does** — Metadata + BLOB; a tombstoned resource reads as `NotFound`
(before touching data).

### fn delete_resource

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn delete_resource`.

**What it does** — Soft delete: tombstone + bumped vector; the payload is
retained (reclaim is `purge_deleted_resources`); tombstone journal row.

### fn list_resources

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn list_resources`.

**What it does** — Live metadata (no BLOBs), `(created_at, id)` keyset.

### fn purge_deleted_resources

**Identification** — marker
`// md:impl ResourceRepository for DbBackend > fn purge_deleted_resources`.

**What it does** — `UPDATE … SET data = NULL` for tombstones older than the
cutoff — frees the dead bytes but keeps the tombstone row (`deleted_at`, vv,
`last_writer`) so the deletion keeps converging; `size` remains as a record.

---

## impl SyncBackend for DbBackend

**Identification** — marker `// md:impl SyncBackend for DbBackend`; per-method
markers `> fn <name>`.

**What it does** — the journal + WebSocket sync surface.

### fn get_changes_since

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn get_changes_since`.

**What it does** — Journal rows with `changed_at > since` in insertion order
(`ORDER BY id`), each mapped through `row_to_change`; unknown rows are logged
and skipped, never abort the sync.

### fn apply_change

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn apply_change`.

**What it does** — Applies one relayed change under the exclusive lock.
**Deliberately does not `record_change`**: the journal holds only changes that
*originated* on this device, so `get_changes_since` never re-sends something
merely received — the relay is a broadcast (it forwards each device's change to
every other peer), so re-journaling would echo every change back out each
cycle. Do not add `record_change` here without also switching the relay away
from broadcast. Per variant, everything is version-vector gated
(`incoming_wins`/`assoc_incoming_wins`/`resource_incoming_wins` — a losing or
equal-vector change is a silent idempotent no-op):

- **Note create/update** — winner ⇒ an atomic transaction refreshing the
  `note_links` projection and `INSERT OR REPLACE`-ing the row (so a crash
  mid-apply cannot desync the index; still idempotent on retry).
- **Note/notebook/tag/resource delete** — winner ⇒ stamp the tombstone; if the
  entity is **unknown locally** (out-of-order delivery), insert a minimal
  tombstone row so a later stale create/update loses in `resolve` instead of
  resurrecting it (issue #71).
- **Notebook/tag create/update** — winner ⇒ `INSERT OR REPLACE`.
- **NoteTagAdd/Remove** — winner ⇒ `upsert_assoc` present/tombstone.
- **ResourceCreate** — winner ⇒ `INSERT OR REPLACE` storing the carried
  payload (empty when the change was blob-stripped).

### fn get_last_sync_time

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn get_last_sync_time`.

**What it does** — `sync_state['last_sync']`, epoch when never synced.

### fn update_sync_time

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn update_sync_time`.

**What it does** — `INSERT OR REPLACE` of the watermark.

### fn send_changes

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn send_changes`.

**What it does** — Empty batch → Ok. No `server_url` → Ok (deliberately
local-only; nowhere to send is not a failure). Otherwise one
`{"type":"changes","batch_id","device_id","changes"}` frame, retried up to 4
attempts with 2/4/8 s backoff; a failed send clears the slot for `ensure_ws`.
If the connection cannot be (re-)established, **fail fast with an error** —
returning Ok would advance the watermark past changes the relay never saw,
silently dropping them forever; the same batch is re-collected next cycle.
`batch_id` + `device_id` drive the server's `(user, batch, index)` dedup.

### fn receive_changes

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn receive_changes`.

**What it does** — Ensure/reconnect (no connection → empty vec), then drain
buffered frames with a 100 ms silence timeout (bounded-time — later messages
arrive next cycle) and a hard cap of 1 000 messages per call (a misbehaving
server cannot exhaust memory; the remainder is delivered next cycle). Malformed
frames are logged and skipped (one bad frame must not block well-formed batches
or fail the cycle); `{"type":"changes"}` frames contribute their batch; a Close
frame or stream error clears the slot for reconnect.

### fn prune_change_journal

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn prune_change_journal`.

**What it does** — `DELETE FROM entity_changes WHERE changed_at < cutoff`,
returning the row count.

### fn get_device_id

**Identification** — marker
`// md:impl SyncBackend for DbBackend > fn get_device_id`.

**What it does** — The cached installation id.

---

## ServerVersion

**Identification** — private deserialise struct; marker `// md:ServerVersion`.

**What it does** — One version as served by keeplin-srv's history endpoints
(`GET /api/{notes,notebooks}/:id/history`): the edit's instant, the authoring
sync device, and the snapshot exactly as pushed (`None` = tombstone). Encrypted
fields are still ciphertext here; `EncryptedBackend` decrypts on the way up,
same as for the local journal.

**Used by** — `server_entity_history`.

---

## CapabilityCache

**Identification** — private enum; marker `// md:CapabilityCache`.

**What it does** — Cached `GET /version` outcome (keeplin#114): `Unknown` (not
fetched — a lazy probe may retry), `Unavailable` (no `/version`; capabilities
indeterminate), `Known(Vec<String>)`.

**Used by** — the `server_capabilities` field, `server_has_capability`.

---

## fn http_base_of

**Identification** — `fn http_base_of(server_url: &str) -> Option<String>`;
marker `// md:fn http_base_of`.

**What it does** — Derives the HTTP base from the WebSocket URL (`ws`→`http`,
`wss`→`https`, the `/api/sync` relay path stripped); `None` for empty or
non-WebSocket URLs (offline). A free function so `DbBackend::new` can run the
handshake before `self` exists.

**Used by** — `new`, `server_http_base`.

---

## impl DbBackend (server history)

**Identification** — the second inherent impl; marker
`// md:impl DbBackend (server history)`. Four methods.

### fn server_http_base

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_http_base`.

**What it does** — `http_base_of(&self.server_url)`.

### fn server_has_capability

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_has_capability`.

**What it does** — Whether the server advertises `capability` at
`GET /version`, fetched once and cached: `Some(true/false)` when the server has
`/version`; `None` when it doesn't (older server) — the caller falls back to
feature-specific probing.

**Used by** — `server_entity_history`.

### fn server_entity_history

**Identification** — marker
`// md:impl DbBackend (server history) > fn server_entity_history`.

**What it does** — Fetches an entity's history from the server (the durable
**cross-device** record). `None` (→ local fallback) when: the 404 latch is set;
capability negotiation says the server lacks `history` (which also sets the
latch); no server configured; a transient network error (does **not** latch);
any HTTP error; malformed JSON. A definitive 404 latches
`history_unsupported` so future reads skip the round-trip (issue #113).
Unparseable snapshots are skipped rather than mislabelled as deletes.

### fn entity_history

**Identification** — marker
`// md:impl DbBackend (server history) > fn entity_history`.

**What it does** — Past versions newest-first: server journal first (a fresh
device sees every device's history, cross-device rollback works), local
`entity_changes` fallback (this device's own changes only). `limit = 0` →
`DEFAULT_HISTORY_LIMIT`. Local mapping: create/update → snapshot (unparseable
→ skip), delete → `entity: None`.

**Used by** — the `HistoryRepository` impl.

---

## impl HistoryRepository for DbBackend

**Identification** — marker `// md:impl HistoryRepository for DbBackend`;
per-method markers `> fn <name>`.

**What it does** — thin typed wrappers.

### fn note_history

**Identification** — marker
`// md:impl HistoryRepository for DbBackend > fn note_history`.

**What it does** — `entity_history::<Note>("note", …)`.

### fn notebook_history

**Identification** — marker
`// md:impl HistoryRepository for DbBackend > fn notebook_history`.

**What it does** — `entity_history::<Notebook>("notebook", …)`.

---

## mod migration_tests

**Identification** — `#[cfg(test)] mod migration_tests`; marker
`// md:mod migration_tests`. Two helpers + four tests.

**What it does** — Pins the migration framework and the journal-derived
history.

### fn raw_conn

**Identification** — helper; marker `// md:mod migration_tests > fn raw_conn`.

**What it does** — A raw libsql connection bypassing `DbBackend::new`, so a
test can plant a pre-framework schema.

### fn user_version

**Identification** — helper; marker
`// md:mod migration_tests > fn user_version`.

**What it does** — Reads the stamp via `schema_version`.

### fn note_history_reads_this_devices_versions_newest_first

**Identification** — tokio test; marker
`// md:mod migration_tests > fn note_history_reads_this_devices_versions_newest_first`.

**What it does** — create + update → two versions newest-first; a delete adds
a tombstone version (`entity: None`) on top; `limit = 1` caps the reply.

### fn fresh_database_is_stamped_current_and_reopen_is_a_noop

**Identification** — tokio test; marker
`// md:mod migration_tests > fn fresh_database_is_stamped_current_and_reopen_is_a_noop`.

**What it does** — A fresh database is stamped `SCHEMA_VERSION`, a note
round-trips, and reopening runs no migrations while preserving data.

### fn migrates_a_pre_framework_database_without_losing_data

**Identification** — tokio test; marker
`// md:mod migration_tests > fn migrates_a_pre_framework_database_without_losing_data`.

**What it does** — Plants an old-shape unstamped `notes` table with a row;
opening through `DbBackend` migrates in place to the current stamp; the legacy
row survives (empty vv, NULL notebook moved to the Inbox, sentinel sort key,
listed under the Inbox) and new writes work.

### fn refuses_to_open_a_newer_schema

**Identification** — tokio test; marker
`// md:mod migration_tests > fn refuses_to_open_a_newer_schema`.

**What it does** — A database stamped `SCHEMA_VERSION + 1` is refused with the
"newer than this build" `InvalidState`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `DbBackend` — defined here (EXTRACTED; the database backend root)
- the repository-trait implementations (implements×6) and the row/versioning helpers (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: references×69)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: calls×2, references×32)
- `keeplin-core/src/links.rs` — `Bookmark`/`NoteLink` column (de)serialisation (EXTRACTED: references×4)
- `keeplin-core/src/storage/backend.rs` — the trait family (EXTRACTED: implements×6, references×8)
- `keeplin-core/src/storage/note_log.rs` — `resolve`/`VersionVector`/`Winner` (EXTRACTED)
- `keeplin-core/src/compat.rs` — the `/version` handshake (INFERRED: fully-qualified paths)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/main.rs` — `build_storage` (INFERRED)
- `keeplin-core/tests/db_backend.rs`, `tests/ws_sync.rs`, `tests/sync.rs`, `tests/migrate.rs` — integration tests (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `type WsStream` | `// md:WsStream` |
| 3 | `struct DbBackend` | `// md:DbBackend` |
| 4 | first `impl DbBackend` (constructor + 34 helpers, incl. `SCHEMA_VERSION`) | `// md:impl DbBackend` (+ `> fn …` per method) |
| 5 | `fn parse_cursor` … `fn tombstone_from_data` (12 free helpers) | `// md:fn <name>` each |
| 6 | `impl NoteRepository for DbBackend` | `// md:impl NoteRepository for DbBackend` (+ `> fn …`) |
| 7 | `impl NotebookRepository for DbBackend` | `// md:impl NotebookRepository for DbBackend` (+ `> fn …`) |
| 8 | `impl TagRepository for DbBackend` | `// md:impl TagRepository for DbBackend` (+ `> fn …`) |
| 9 | `impl ResourceRepository for DbBackend` | `// md:impl ResourceRepository for DbBackend` (+ `> fn …`) |
| 10 | `impl SyncBackend for DbBackend` | `// md:impl SyncBackend for DbBackend` (+ `> fn …`) |
| 11 | `struct ServerVersion` | `// md:ServerVersion` |
| 12 | `enum CapabilityCache` | `// md:CapabilityCache` |
| 13 | `fn http_base_of` | `// md:fn http_base_of` |
| 14 | second `impl DbBackend` (server history) | `// md:impl DbBackend (server history)` (+ `> fn …`) |
| 15 | `impl HistoryRepository for DbBackend` | `// md:impl HistoryRepository for DbBackend` (+ `> fn …`) |
| 16 | `mod migration_tests` (+ 2 helpers + 4 tests) | `// md:mod migration_tests` (+ `> fn …`) |

Note: the `SCHEMA_VERSION` const inside the first impl is covered by the
`fn run_migrations` section (it is that machinery's version pin) and carries no
separate marker.
