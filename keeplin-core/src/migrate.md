# `migrate.rs` — one-shot state copy between backends

## Purpose

Copy the **complete current state** of one `StorageBackend` into another, in either
direction, so a store can be moved between the filesystem backend (`FsBackend`) and the
database backend (`DbBackend`) — including across an encryption boundary. It is a one-shot
migration, **not** live sync: after it runs, each backend keeps using its own native
replication (Syncthing for FS, WebSocket for DB).

## Why a dedicated path (not `get_changes_since`/`apply_change`)

The two backends have **asymmetric** sync channels, so the raw `Change` interface is not
interchangeable between them:

- `FsBackend::get_changes_since` reads only the global NDJSON journal (notebooks/tags/
  resources) — notes live in per-note version-vector logs and are **not** emitted there.
- `FsBackend::apply_change` for a note **ignores the payload** and only re-materializes logs
  already on disk (the Syncthing assumption), so importing a note it hasn't already received
  is a silent no-op.
- `EncryptedBackend` passes `apply_change` through **without encrypting**, so it can't be the
  destination of a raw change either.

`migrate` sidesteps all three by copying through the typed **`create_*` methods**, which every
layer implements correctly (real VV log on FS, indexed row on DB, encrypt-on-write when wrapped).

## Public API

### `fn migrate(src: &dyn StorageBackend, dst: &dyn StorageBackend) -> Result<MigrationReport>`

Copies every live entity from `src` to `dst`, in dependency order so references resolve as
entities land:

| Order | Entity | How |
|-------|--------|-----|
| 1 | notebooks | `list_notebooks` → `dst.create_notebook` |
| 2 | tags | `list_tags` → `dst.create_tag` |
| 3 | notes | `list_notes` → `dst.create_note` (`alias`/`bookmarks`/`links` ride along as fields) |
| 4 | note↔tag | per note, `src.list_note_tags` → `dst.add_note_tag` |
| 5 | resources | `list_resources` + `src.read_resource` (bytes) → `dst.create_resource` |

Returns a `MigrationReport { notebooks, tags, notes, note_tags, resources }` of per-entity
counts. On the DB destination, `create_note` rebuilds the `note_links` backlink index from the
copied `links`; when either side is an `EncryptedBackend`, reads decrypt and writes encrypt, so
each side uses its own key.

### `struct MigrationReport`

Per-entity counts of what was copied (`Debug`, `Default`, `Copy`, `Eq`).

## Helper

`collect` exhausts any paginated `list_*` closure (`Option<token> -> (items, next)`) into a
`Vec`, so one helper drives all five entity kinds.

## Scope (deliberate limitations)

- **Live state only.** `list_*` exclude soft-deleted rows, so tombstones are not carried — a
  migration is a fresh start.
- **Fresh destination.** Entities keep their original ids; `DbBackend::create_note` is a plain
  `INSERT`, so importing an existing id errors. Migrate into an empty destination.
- Fails fast on the first error, leaving already-written entities in place.

## How it's invoked

The daemon exposes it as `keeplin-daemon migrate --from <a.toml> --to <b.toml>`, building each
side from its own config (see `keeplin-daemon/src/main.md`).

## Related files

- `keeplin-core/src/storage/backend.rs` — the `create_*` / `list_*` / `read_resource` methods used.
- `keeplin-core/src/storage/{fs,db}.rs` — the two backends being bridged.
- `keeplin-core/src/encryption.rs` — the decorator that makes encrypted↔plaintext copies work.
- `keeplin-daemon/src/main.rs` — the `migrate` subcommand and `build_storage`.
- `keeplin-core/tests/migrate.rs` — FS↔DB round-trips and the encrypted case.
