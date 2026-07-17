# `tests/migrate.rs` — cross-backend migration tests

## What is tested

Integration tests for `keeplin_core::migrate::migrate` — the one-shot copy of all live state
from one backend into another. Each test seeds a source backend with one of every entity type
(a note with alias/bookmarks/links, a notebook, a tag, a note↔tag association, and a binary
resource), runs `migrate`, and asserts the destination holds a faithful copy. The seed is
written **directly** through the backend (no `LinkingBackend`), so the tests check verbatim
field fidelity rather than link re-derivation.

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `fs_to_db_round_trip` | Seed an `FsBackend`, migrate into a `DbBackend` | Every entity and field round-trips; the copy reads back identically |
| `db_to_fs_round_trip` | The reverse direction | Same fidelity `DbBackend` → `FsBackend` |
| `encrypted_fs_to_encrypted_db` | Both sides wrapped in `EncryptedBackend` (different keys per side) | Plaintext migrates correctly even though the ciphertexts differ; each side encrypts with its own key |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `seed(src)` | Write one of every entity type and return a `Seeded` record of the ids/values to check |
| `assert_migrated(dst, seeded)` | Assert the destination reproduces every seeded entity and field |
| `db(dir)` | Build a `DbBackend` on a temp path with no server URL (offline) |

## Coverage gaps

- Migration is a **one-shot copy of current live state**, not live sync, so conflict
  resolution and tombstone propagation are covered by `sync.rs` / `ws_sync.rs` /
  `db_backend.rs`, not here.
- Notebook membership migrates as-is; the Inbox/ordering placement rules are exercised in the
  `ordering` unit tests, not through `migrate`.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `seed()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `assert_migrated()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `db()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `fs_to_db_round_trip()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `db_to_fs_round_trip()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `encrypted_fs_to_encrypted_db()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `Seeded` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/migrate.rs` — one-shot state copy between backends (EXTRACTED: calls×3; e.g. `migrate()`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×2; e.g. `StorageBackend`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×1; e.g. `DbBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Seeds one of every entity type and asserts a faithful copy in the destination — new entity kinds must be added to the seed when introduced.
- Covers crossing the encryption boundary in both directions.

## Related files

- `keeplin-core/src/migrate.rs` — the code under test.
- `keeplin-core/tests/sync.rs`, `keeplin-core/tests/ws_sync.rs` — the live-sync paths that
  migration deliberately does not cover.
