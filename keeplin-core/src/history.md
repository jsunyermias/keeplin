# `history.rs` — change history reads + forward-revert

## Purpose

User-facing **version history** and **roll-back** for notes and notebooks. History is not a
separate store: it is **derived from the change journal** each backend already keeps, because
every journalled change carries a full entity snapshot (`Change::NoteUpdate { note }`). This
module adds the roll-back operations on top of the [`HistoryRepository`] read capability, as
free functions over a type-erased `&dyn StorageBackend`.

## Where the history comes from

| Backend | Source of versions | Depth |
|---------|--------------------|-------|
| `FsBackend` (notes) | the per-note per-device op logs (`log.{device}.msgpack`) — each entry is one version | up to `NOTE_LOG_COMPACT_THRESHOLD` (256) before the log collapses to its frontier |
| `FsBackend` (notebooks) | the global NDJSON journal (`logs/*.log`) | best-effort — that journal compacts to current state, so deep notebook history is not retained |
| `DbBackend` | the `entity_changes` table (this device's originated changes) | full until `prune_change_journal` trims it |

Because at-rest encryption sits **below** the backend, the journal holds ciphertext snapshots;
`EncryptedBackend` decrypts each version on the way up, so a history read returns plaintext —
unlike `get_changes_since`, which passes ciphertext through for the sync relay.

### Server-authoritative durability

In `DbBackend` mode the server (keeplin-srv) is the source of truth and holds every device's
changes; the local `entity_changes` journal is this device's own contributions (a cache). Full
cross-device history is served from the server — a follow-up stage. In `FsBackend` mode the
per-note logs are replicated by Syncthing, so history is already multi-device.

## Retention

Governed by the journal's own bounds, matching the chosen **count + age** policy:

- **count** — `FsBackend` keeps up to 256 versions per note before compaction; `DbBackend`/the
  server keep every change until pruned.
- **age** — keeplin-srv's `journal_retention_days` prunes the change journal by age; set it to
  `0`/disabled to keep history by **count only**.

## Forward revert (non-destructive)

`revert_*` never deletes intervening versions. It writes the target state back as a **new**
edit, so `update_note`/`update_notebook` mint a fresh version vector that dominates everything
seen so far — the revert converges under sync like any edit and can itself be undone by
reverting again. A version that was a **tombstone** at the target instant reverts to a delete.

## "As of" semantics

Every revert targets an **instant**, not an opaque version id: the state as of `at` is the
newest version with `timestamp <= at` ([`state_at`], a pure helper). Reverting one version is
just reverting to that version's own timestamp; point-in-time and **batch** rollback (a whole
notebook back to before a bad change) fall out of the same primitive.

## Public API

| Function | Description |
|----------|-------------|
| `state_at(versions, at)` | pure: newest version at or before `at` (`versions` newest-first) |
| `revert_note(be, id, at)` | forward-revert one note; returns the resulting note |
| `revert_notebook(be, id, at)` | forward-revert one notebook |
| `revert_notes_to(be, ids, at)` | batch forward-revert an explicit id list |
| `revert_notebook_notes_to(be, notebook_id, at)` | batch-revert every note **currently** in a notebook |

The read side is [`HistoryRepository::note_history`] / `notebook_history` (newest first,
`limit = 0` → `DEFAULT_HISTORY_LIMIT`).

## Related files

- `keeplin-core/src/storage/backend.rs` — the `HistoryRepository` trait and `EntityVersion<T>`.
- `keeplin-core/src/storage/fs.rs` / `db.rs` — the two journal-derived implementations.
- `keeplin-core/src/encryption.rs` — decrypts each historical snapshot.
- `keeplin-daemon/src/rest.rs` — the `history`/`revert` HTTP endpoints.
