# Keeplin — Architecture overview

This document is the **one-page mental model** for the whole project. Every other `.md`
(and every source file's `*.md` companion) drills into one piece; read this first to see how
they fit together. Repetition with those files is intentional — this page should stand alone.

---

## 1. What Keeplin is

A local-first note-taking **daemon**. Notes, notebooks, tags, and binary resources live on the
user's own device and are replicated between the user's devices — either by copying files with
an external tool (Syncthing) or by talking to a sync server over WebSocket. Clients talk to the
daemon through **gRPC**, a **REST/JSON** API, and a **WebSocket live-change feed**.

Two crates:

| Crate | Kind | Contains |
|-------|------|----------|
| `keeplin-core` | library | domain models, the two storage backends, at-rest encryption, the bookmark/link layer, and the sync engine |
| `keeplin-daemon` | binary | the gRPC + REST + WebSocket servers, auth, config, and the process wiring |

---

## 2. The domain model (`keeplin-core/src/models.rs`)

Everything is one of a handful of plain structs, all deriving `Serialize`/`Deserialize`:

- **`Note`** — `title`, `body`, optional `notebook_id`, to-do fields, timestamps, and the
  navigation fields **`alias`**, **`bookmarks`**, **`links`** (see section 6).
- **`Notebook`**, **`Tag`** — a title + timestamps; notebooks also have an **`alias`**.
- **`NoteTag`** — a (note, tag) association.
- **`Resource`** — metadata for a binary attachment (the bytes are stored separately).
- **`Change`** — the unit of sync: one enum variant per mutation (`NoteCreate`, `NoteDelete`,
  `NoteTagAdd`, `ResourceCreate`, …). Replaying a device's `Change`s reproduces its state.

Every entity — notes, notebooks, tags, note↔tag associations, and resources — uses **soft
delete** (`deleted_at` is set, the record is kept as a versioned tombstone for sync). A resource's
binary payload is retained after a soft delete; reclaiming that space is left to the `FsBackend`
compaction phase.

---

## 3. The storage trait and the two backends

`StorageBackend` (`storage/backend.rs`) is a **supertrait of five focused sub-traits** —
`NoteRepository`, `NotebookRepository`, `TagRepository`, `ResourceRepository`, `SyncBackend`.
A blanket impl means any type implementing all five *is* a `StorageBackend`, so
`Arc<dyn StorageBackend>` is usable everywhere. Two concrete backends:

| Backend | Storage | Conflict resolution | Sync transport |
|---------|---------|---------------------|----------------|
| **`FsBackend`** (`storage/fs.rs`) | files under a root dir | **version vectors** (per-note logs merged; sidecar entities resolved by `note_log::resolve`) | passive — an external tool (Syncthing) replicates the files |
| **`DbBackend`** (`storage/db.rs`) | one local LibSQL/SQLite database | **version vectors** (current-state rows resolved by `note_log::resolve`) | active — WebSocket to a sync server |

Both backends resolve **every** entity — notes, notebooks, tags, note↔tag associations, and
resources — through version vectors with the same deterministic `(timestamp, device_id)` tiebreak,
so every device converges. The storage shapes differ (append-only per-device logs vs. current-state
rows) but the decision is shared; see `SECURITY.md`.

---

## 4. The decorator stack — the key idea

Behaviour is layered as **decorators**, each of which is itself a `StorageBackend` wrapping an
inner one. The daemon builds this stack at startup (innermost → outermost):

```
        EventBackend            ← publishes every mutation to the WebSocket feed (daemon)
          └ LinkingBackend      ← derives bookmarks/links, resolves refs, enforces alias uniqueness (core)
              └ [EncryptedBackend]  ← optional AES-256-GCM at-rest encryption (core)
                  └ FsBackend | DbBackend   ← actual persistence (core)
```

Why this order:

- **`LinkingBackend` is outside `EncryptedBackend`** so it parses the **plaintext** body (to
  find bookmarks and links) and resolves aliases against **decrypted** reads.
- **`EventBackend` is outside `LinkingBackend`** so the live feed carries the fully-refreshed
  note (with derived bookmarks/links).
- Both the gRPC server and the REST server share **one** `Arc` to the top of this stack, so a
  mutation from either surface flows through every layer once.

---

## 5. Encryption (`keeplin-core/src/encryption.rs`)

`EncryptedBackend<B>` transparently encrypts the sensitive, human-readable fields
(`Note.title`/`body`/`alias`, each bookmark's `text`/`alias`, each link's `raw`,
`Notebook.title`/`alias`, `Tag.title`, `Resource.title`/`mime_type`/`file_name`, and the binary
payload) with **AES-256-GCM**; the key is derived from a password with **Argon2id**. UUIDs,
timestamps, sizes, and a link's resolved `target_note_id` stay **plaintext** because they are
needed for indexing and sync and carry no user content. Every value gets a fresh random nonce.

---

## 6. Bookmarks and links (`links.rs` + `linking.rs`)

Two navigation features layered on notes, both **stored on the note** (so they ride the normal
`Change` sync path — no new `Change` variants):

- **Bookmarks** — in-note anchors declared in the body as a markdown link whose destination is
  exactly `###`: `[text](### "alias")`. The link text is the bookmark `text`, the optional
  title is its `alias` (default = text), and its `number` is its order of appearance. The
  **body is the single source of truth** — there is **no bookmark API**; bookmarks are created,
  renamed, and removed by editing the note body, and are returned inline in each note's
  `bookmarks` field.
- **Links** — connections to other notes: **content-derived** from markdown links `[t](#…)`, or
  **manual** (added via the API). A reference resolves as `#note`, `#notebook#note`, or
  `#notebook#note#bookmark`; each segment is alias-or-uuid (the bookmark segment is
  alias-or-number).

`LinkingBackend` re-derives bookmarks/content-links on every note write, resolves each link's
`target_note_id`, and enforces that note/notebook aliases are unique.

---

## 7. Sync (`keeplin-core/src/sync/engine.rs`)

`run_sync` drives one cycle: collect local `Change`s since the last sync watermark → send →
receive remote `Change`s → apply each (`apply_change` is **idempotent**) → record the new
watermark → optionally prune old journal entries. `FsBackend` "sends" passively (Syncthing
copies its logs); `DbBackend` sends/receives over WebSocket with retry. The two backends'
`Change` channels are **not** interchangeable for live sync, so they don't cross-sync directly.

**Migration** (`keeplin-core/src/migrate.rs`) is the one-shot escape hatch: `migrate(src, dst)`
copies all live state between any two backends via the typed `create_*` methods (not
`apply_change`), so `Fs ↔ Db` and plaintext ↔ encrypted all work. Exposed as
`keeplin-daemon migrate --from a.toml --to b.toml`.

---

## 8. The surfaces (`keeplin-daemon`)

| Surface | File | Notes |
|---------|------|-------|
| **gRPC** | `proto/keeplin.proto` + `src/server.rs` | full CRUD + sync + bookmark/link RPCs; HTTP Basic-Auth metadata |
| **REST/JSON** | `src/rest.rs` (axum) | same operations over JSON on an optional `http_addr`; body capped at `max_message_size` |
| **WebSocket feed** | `src/rest.rs` + `src/event_backend.rs` | `GET /api/ws` streams every `Change` as JSON |

All three share one backend `Arc` and **one auth model**: a constant-time HTTP Basic check in
`src/auth.rs`, used by both the gRPC interceptor and the axum middleware.

---

## 9. Where to read next

- Storage internals: `keeplin-core/src/storage/{backend,fs,db,note_log}.md`.
- Migration between backends: `keeplin-core/src/migrate.md`.
- Encryption + threat model: `keeplin-core/src/encryption.md` and `SECURITY.md`.
- Bookmarks/links: `keeplin-core/src/links.md`, `keeplin-core/src/linking.md`.
- Surfaces: `keeplin-daemon/src/{server,rest,event_backend,auth}.md` and
  `keeplin-daemon/proto/keeplin.proto.md`.
- User-facing overview and API tables: `README.md`.
