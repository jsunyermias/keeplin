# `event_backend.rs` — `EventBackend` change-publishing decorator

## Purpose

The outermost decorator in the backend stack. It wraps any `StorageBackend` and, after every
**successful** mutation, publishes the corresponding `Change` to a `tokio::sync::broadcast`
channel. The daemon's WebSocket route (`GET /api/ws`) subscribes to that channel and streams
each `Change` to connected clients — a **live feed of changes**.

## Placement in the stack

Outermost, so it sits above `LinkingBackend` and any `EncryptedBackend`:

```
EventBackend( LinkingBackend( [EncryptedBackend]( Fs | Db ) ) )
```

Consequences of this position:

- It publishes the value **returned** by the inner layers, i.e. the fully-refreshed note
  (with derived bookmarks/links) and, because it is above encryption, **plaintext**. WebSocket
  clients receive decrypted data — the daemon is the trust boundary.
- Because both the gRPC service and the REST API share this one instance, a mutation from
  **either** surface emits exactly one event.

## What it publishes

| Mutation | Published `Change` |
|----------|--------------------|
| `create_note` / `update_note` | `NoteCreate` / `NoteUpdate` with the stored note |
| `delete_note` | `NoteDelete { id, deleted_at: now }` |
| notebook / tag create/update/delete | the matching `Notebook*` / `Tag*` variant |
| `add_note_tag` / `remove_note_tag` | `NoteTagAdd` / `NoteTagRemove` |
| `create_resource` / `delete_resource` | `ResourceCreate { data: None }` / `ResourceDelete` |

Read methods and the sync methods delegate without publishing. Resource creates are streamed as
metadata only (`data: None`); a client fetches the bytes via `GET /api/resources/:id/data`.

## Delivery semantics

The channel is **lossy and best-effort**: a subscriber that falls behind the channel capacity
sees a `Lagged` error, which the WebSocket route turns into a `{"type":"resync"}` hint. The
feed is a notification stream, not a durable log — the authoritative history is the per-device
change journal used by sync. Publishing never blocks a mutation: a send with no live receivers
just returns an ignored error.

## Construction

`EventBackend::new(inner, tx)` where `tx` is a `broadcast::Sender<Change>` created once in
`main.rs`. `main.rs` keeps a clone of the same `Sender` in the REST `AppState` so the WebSocket
route can `subscribe()` to it.

## Design notes

- Publishing after the inner call **succeeds** means a failed/rejected mutation emits nothing.
- Living in the daemon (not core) keeps the broadcast/WebSocket concern out of the storage
  library; it mirrors the core decorator pattern (`EncryptedBackend`, `LinkingBackend`).

## Related files

- `keeplin-daemon/src/rest.rs` — the `GET /api/ws` route that subscribes and streams frames.
- `keeplin-daemon/src/main.rs` — creates the channel and wires the stack.
- `keeplin-core/src/models.rs` — the `Change` enum that is published.
