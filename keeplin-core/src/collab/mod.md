# `collab/mod.rs` — client of the keeplin-srv collaborative channel

Self-contained companion for `keeplin-core/src/collab/mod.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the child-module declarations and imports.
Marker `// md:Overview`.

```rust
pub mod protocol;
pub mod state;
// … std/tokio/tungstenite/reqwest imports, crate error/model/storage types,
// protocol::{CollabClientMsg, CollabServerMsg}, state::NoteLines
```

**What it does** — The client side of keeplin-srv's collaborative channel.
Architecture: frontends talk to the daemon (gRPC/REST/feed); the daemon — through
`CollabBackend` — talks to keeplin-srv. The server stores every note (lines +
versioned order + metadata) and is the durable source of truth; this client keeps
the user's notes (own and shared) in the local database and mirrors line state in
memory, rebuilt from each `Welcome` snapshot on (re)connect.

- Local note writes are diffed into `protocol::LineOp`s (signed with this
  **device**'s id, the vv actor) and pushed over the WebSocket; title/metadata
  changes go over REST.
- Remote ops and snapshots are applied through the daemon's full decorator stack
  (so links are re-derived and the live feed fires), with an in-flight
  **suppression set** preventing those writes from echoing back into ops.
- Note discovery (own + shared) is a paged REST `GET /api/notes` at connect,
  following the server's `X-Next-Cursor` header.
- Note `Change`s are **filtered out of the relay sync path**: the collab channel
  owns notes; notebooks/tags/resources keep syncing via the relay.

**Dependencies** — `tokio`/`tokio_tungstenite`/`futures_util` (WebSocket),
`reqwest` (REST), `serde_json`, `base64`, `anyhow` (connection task); the sibling
`protocol`/`state` modules; the crate's error/model/storage-trait types;
`crate::compat` for the handshake.

**Used by** — `keeplin-daemon/src/main.rs` (constructs and starts it in server
mode), `keeplin-daemon/src/rest.rs` (heavily — it is the note surface in server
mode plus the presence/permission proxy), `keeplin-core/tests/collab_client.rs`.

**Repeated context** — Core invariants: (1) the relay must never carry note
`Change`s or a note would travel both paths and double-apply; (2) server-driven
writes go through `top` with the id suppressed — no echo; (3) a note pending its
first `Welcome` is reconciled against the snapshot, never pushed eagerly — a late
empty `Welcome` must not clobber local content; (4) resource binaries never ride
the relay journal; (5) `start` runs the `compat` handshake first — an
incompatible server spawns no connection task.

---

## CollabConfig

**Identification** — struct deriving `Debug, Clone`; marker
`// md:CollabConfig`.

**What it does** — Connection settings: `api_url` (HTTP base, e.g.
`http://host:3000`), `ws_url` (e.g. `ws://host:3000/api/ws`), `token` (device
token from keeplin-srv `POST /api/login` — one per device).

**Dependencies** — none.

**Used by** — `CollabBackend::new`; built by `keeplin-daemon/src/main.rs` from
config.

**Repeated context** — none.

---

## fn device_id_from_token

**Identification** — `pub fn device_id_from_token(token: &str) -> Option<String>`;
marker `// md:fn device_id_from_token`.

**What it does** — Extracts the `device_id` claim from a JWT **without
verifying** it: split on `.`, base64url-decode the payload, read
`claims.device_id`. The server verifies tokens; the client only needs to know
its own identity to sign ops (the vv actor).

**Dependencies** — `base64`, `serde_json`.

**Used by** — `CollabBackend::new` (fails construction with `InvalidState` when
the claim is missing).

**Repeated context** — device-as-vv-actor: every op's `last_writer` and
advancing vv component is this device id, and the server rejects mismatches
(`bad_writer`).

---

## Shared

**Identification** — private `struct Shared`; marker `// md:Shared`.

**What it does** — The state shared between the decorator, the handle, and the
connection task: `cfg`, `device_id`, a `reqwest::Client`,
`notes: Mutex<HashMap<Uuid, NoteLines>>` (per-note in-memory mirror),
`suppress: Mutex<HashSet<Uuid>>` (ids currently being written *from* the server
so the decorator doesn't diff them back — no echo),
`presence: Mutex<HashMap<Uuid, Vec<PresenceInfo>>>` (latest per-note presence,
served to frontends via the daemon's REST API), `out` (unbounded outbound queue
drained by the connection task), `pending_push: Mutex<HashSet<Uuid>>` (notes
with local content not yet on the server — freshly created, or edited before the
first `Welcome`; their body is reconciled against the join snapshot instead of
pushed eagerly, so a late empty `Welcome` cannot clobber the local content),
`top: OnceLock<Arc<dyn StorageBackend>>` (the daemon's outermost decorator, set
once by `start`, used to apply remote state so linking/eventing run on those
writes too).

**Dependencies** — `NoteLines`, `CollabClientMsg`, tokio sync primitives.

**Used by** — everything in this file.

**Repeated context** — none.

---

## impl Shared

**Identification** — inherent impl; marker `// md:impl Shared`. Five methods.

### fn auth

**Identification** — `fn auth(&self, req) -> reqwest::RequestBuilder`; marker
`// md:impl Shared > fn auth`.

**What it does** — Adds `Authorization: Bearer <token>` to a request builder.

**Dependencies** — `reqwest`.

**Used by** — every REST call in the file.

**Repeated context** — the token never travels in URLs (query strings end up in
proxy and access logs).

### fn suppressed

**Identification** — `async fn suppressed(&self, id: Uuid) -> bool`; marker
`// md:impl Shared > fn suppressed`.

**What it does** — Whether `id` is currently in the suppression set.

**Dependencies** — `suppress`.

**Used by** — the `NoteRepository` write methods.

**Repeated context** — none.

### fn apply_from_server

**Identification** —
`async fn apply_from_server(&self, note: Note, create: bool) -> Result<(), StorageError>`;
marker `// md:impl Shared > fn apply_from_server`.

**What it does** — Writes `note` through the top of the stack with echo
suppression: insert the id into `suppress`, `create_note` or `update_note` on
`top`, remove the id. A no-op (Ok) before `start` set `top`.

**Dependencies** — `top`, `suppress`.

**Used by** — `ensure_local`, `write_body`.

**Repeated context** — going through `top` (not `inner`) is what makes remote
writes re-derive links and fire the daemon's live feed exactly like local ones.

### fn upload_blob

**Identification** — `async fn upload_blob(&self, id: Uuid, data: Vec<u8>)`;
marker `// md:impl Shared > fn upload_blob`.

**What it does** — Uploads a resource's binary to keeplin-srv out-of-band
(`PUT /api/resources/{id}/data`) so bytes never ride the relay journal. The
server accepts a blob only once the resource's metadata has been
**materialised** from the relay journal — asynchronous after the eager push in
`create_resource` — so a non-success answer (e.g. a 404 while metadata is in
flight) is retried up to 5 times with linear backoff (200 ms × attempt). A
transport failure aborts immediately (the server is unreachable; an immediate
retry cannot help). Best effort beyond that: persistent failure is logged,
metadata still syncs, and the blob can be re-uploaded by a later
create/replace.

**Dependencies** — `auth`, `reqwest`, `tokio::time`.

**Used by** — `create_resource`.

**Repeated context** — none.

### fn download_blob

**Identification** — `async fn download_blob(&self, id: Uuid) -> Option<Vec<u8>>`;
marker `// md:impl Shared > fn download_blob`.

**What it does** — Fetches a resource's binary from keeplin-srv (the source of
truth for blobs); `None` on any failure so the caller falls back to the local
cache.

**Dependencies** — `auth`, `reqwest`.

**Used by** — `read_resource`.

**Repeated context** — none.

---

## CollabHandle

**Identification** — `#[derive(Clone)] pub struct CollabHandle`; marker
`// md:CollabHandle`.

**What it does** — An ungeneric, cloneable view of the collaborative session
for the daemon's HTTP/gRPC surfaces: read presence and publish this device's
cursor without knowing the storage type behind `CollabBackend`.

**Dependencies** — `Shared`.

**Used by** — `keeplin-daemon/src/rest.rs` (presence endpoint, cursor endpoint,
permission proxy).

**Repeated context** — none.

---

## impl CollabHandle

**Identification** — inherent impl; marker `// md:impl CollabHandle`. Three
methods.

### fn presence

**Identification** — `pub async fn presence(&self, note_id: Uuid) -> Vec<PresenceInfo>`;
marker `// md:impl CollabHandle > fn presence`.

**What it does** — The latest presence list the server broadcast for `note_id`
(empty when the note has no live session or is unknown).

**Dependencies** — the `presence` map.

**Used by** — `rest.rs::note_presence`.

**Repeated context** — none.

### fn send_cursor

**Identification** — `pub fn send_cursor(&self, note_id: Uuid, cursor: Cursor)`;
marker `// md:impl CollabHandle > fn send_cursor`.

**What it does** — Queues this device's caret position; the server fans the
updated presence out to every participant. Fire-and-forget (send errors
ignored — the connection task may be between reconnects).

**Dependencies** — the `out` queue.

**Used by** — `rest.rs`'s cursor endpoint.

**Repeated context** — none.

### fn proxy_request

**Identification** —
`pub async fn proxy_request(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<(u16, serde_json::Value), StorageError>`;
marker `// md:impl CollabHandle > fn proxy_request`.

**What it does** — Forwards a **permission-management** request (share,
transfer, list, revoke) to keeplin-srv — the authority for permissions in
server mode — returning the server's status code and JSON body. The daemon's
REST surface proxies its own permission endpoints through this, so a frontend
manages shares without the client re-implementing (or locally enforcing) the
model. A bad verb is `InvalidInput`; a transport failure is
`StorageError::Database`; a non-JSON body becomes `Null`.

**Dependencies** — `auth`, `reqwest`.

**Used by** — `rest.rs`'s share/permission endpoints.

**Repeated context** — permissions are never enforced client-side; the
capability bitset model (READ/WRITE/SHARE_*/MANAGE with the destructive
notebook→note cascade) lives entirely in keeplin-srv.

---

## CollabBackend

**Identification** — `pub struct CollabBackend<B>`; marker
`// md:CollabBackend`.

**What it does** — The storage decorator that turns local note writes into
collaborative ops and REST calls. Sits **below** `LinkingBackend`/`EventBackend`
in the stack. Fields: `inner: Arc<B>`, `shared: Arc<Shared>`,
`out_rx: Arc<Mutex<Option<Receiver<…>>>>` (the outbound queue's receive half,
taken exactly once by `start`).

**Dependencies** — `Shared`, `StorageBackend`.

**Used by** — `keeplin-daemon/src/main.rs` (server-mode stack assembly).

**Repeated context** — none.

---

## impl Clone for CollabBackend

**Identification** — manual impl; marker `// md:impl Clone for CollabBackend`.

**What it does** — Clones the three `Arc`s. Manual because `B` itself need not
be `Clone`.

**Dependencies** — none.

**Used by** — daemon plumbing.

**Repeated context** — none.

---

## impl CollabBackend

**Identification** — inherent impl `impl<B: StorageBackend> CollabBackend<B>`;
marker `// md:impl CollabBackend`. Five methods.

### fn new

**Identification** — `pub fn new(inner: B, cfg: CollabConfig) -> Result<Self, StorageError>`;
marker `// md:impl CollabBackend > fn new`.

**What it does** — Extracts the device id from the token (`InvalidState` when
the claim is missing), creates the outbound channel, and assembles the shared
state with empty mirrors/sets and an unset `top`.

**Dependencies** — `device_id_from_token`, `mpsc`.

**Used by** — `main.rs::build_storage` in server mode.

**Repeated context** — none.

### fn handle

**Identification** — `pub fn handle(&self) -> CollabHandle`; marker
`// md:impl CollabBackend > fn handle`.

**What it does** — A cloneable presence/cursor/proxy view for the daemon's
surfaces.

**Dependencies** — `Shared`.

**Used by** — `main.rs` → `rest.rs` state.

**Repeated context** — none.

### fn start

**Identification** —
`pub async fn start(&self, top: Arc<dyn StorageBackend>) -> Result<(), StorageError>`;
marker `// md:impl CollabBackend > fn start`.

**What it does** — Runs the `GET /version` handshake first (`crate::compat`):
`Compatible` → log negotiated protocol + capabilities and proceed;
`Incompatible` → **loud error** (`InvalidState` with `incompatible_message`)
and the connection task is never spawned (no sync attempted); `Unavailable` →
warn and proceed (backward compatible with older keeplin-srv). Then sets `top`
(must be the outermost backend of the stack so remote writes flow through every
decorator exactly once), takes the outbound receiver (panics if started twice),
and spawns `run_connection`.

**Dependencies** — `compat::negotiate`/`incompatible_message`,
`run_connection`, `tokio::spawn`.

**Used by** — `main.rs` after assembling the stack.

**Repeated context** — none.

### fn push_local_edit

**Identification** — `async fn push_local_edit(&self, note: &Note)`; marker
`// md:impl CollabBackend > fn push_local_edit`.

**What it does** — Diffs `note.body` against the mirror (creating an empty
mirror for an unknown id) via `NoteLines::diff_body` with this device as
actor, and queues the resulting ops (if any) as one `CollabClientMsg::Op`.

**Dependencies** — `state::NoteLines::diff_body`, the `out` queue.

**Used by** — `update_note`.

**Repeated context** — none.

### fn patch_meta

**Identification** — `async fn patch_meta(&self, note: &Note)`; marker
`// md:impl CollabBackend > fn patch_meta`.

**What it does** — Mirrors title/notebook/todo metadata to the server
(`PATCH /api/notes/{id}`). Checks the HTTP status, not just transport success:
a 4xx/5xx means the server rejected the metadata mirror (e.g. an auth or
validation change), which is logged as a warning rather than treated as
delivered (issue #112). Failures are retried implicitly by the next edit — the
server holds the durable truth and the local row already has the change.

**Dependencies** — `auth`, `reqwest`, `serde_json`.

**Used by** — `create_note`, `update_note`.

**Repeated context** — none.

---

## impl NoteRepository for CollabBackend

**Identification** —
`#[async_trait] impl<B: StorageBackend> NoteRepository for CollabBackend<B>`;
marker `// md:impl NoteRepository for CollabBackend` (one marker for the impl
block; the methods are documented here).

**What it does** — The note surface, where local writes become collab traffic:

- `create_note` — inner create; if suppressed (a server-driven write), stop
  there. Otherwise: **mark `pending_push` BEFORE the note becomes visible on
  the server** — once the POST lands, periodic rediscovery can Join the note
  on its own, and if that early Join's (empty) `Welcome` arrived while the
  note was not yet pending, the Welcome handler's cache branch would overwrite
  the fresh local body with the server's empty snapshot, destroying the
  content. Then `POST /api/notes` registering the note under its local id
  (2xx ok; **409 is the expected "already exists"** — created on another
  device — and fine, the body still converges through resolution; any other
  status is logged as a real rejection, issue #112), `patch_meta`, and queue a
  `Join`. The body is pushed when the Join's `Welcome` arrives (reconciled
  against the snapshot), never eagerly; the connection task drops the Join if
  rediscovery already joined (a duplicate Join would fetch a second, possibly
  stale snapshot).
- `update_note` — read the previous row, inner update; if suppressed, done.
  `patch_meta` only when title/notebook/todo fields changed; body ops via
  `push_local_edit` — unless the note is still in `pending_push`, in which
  case the first `Welcome`'s reconcile (which reads the latest local body)
  will push it instead of diffing against an uninitialised mirror.
- `delete_note` — inner delete; unless suppressed, `DELETE /api/notes/{id}`
  (transport failures logged) and drop the mirror entry.
- `read_note`, `list_notes`, `note_backlinks` (explicit delegation so inner
  indexes are reached), `list_notes_in_notebook`, `list_starred_notes`,
  `notebook_sort_profile` — pure delegation.

**Dependencies** — the inner backend, `Shared`, `patch_meta`,
`push_local_edit`.

**Used by** — all note traffic in server mode.

**Repeated context** — the suppression check on every write path is the
no-echo invariant; decorators delegating `note_backlinks` is the
indexed-override rule from `storage/backend.md`.

---

## impl NotebookRepository for CollabBackend

**Identification** — marker `// md:impl NotebookRepository for CollabBackend`.

**What it does** — Pure delegation for all five notebook methods: notebooks
are not collaborative — they sync over the relay.

**Dependencies** — the inner backend.

**Used by** — notebook traffic.

**Repeated context** — none.

---

## impl TagRepository for CollabBackend

**Identification** — marker `// md:impl TagRepository for CollabBackend`.

**What it does** — Pure delegation for all eight tag/association methods (tags
sync over the relay).

**Dependencies** — the inner backend.

**Used by** — tag traffic.

**Repeated context** — none.

---

## impl ResourceRepository for CollabBackend

**Identification** — marker `// md:impl ResourceRepository for CollabBackend`.

**What it does** —

- `create_resource` — inner create, then **eagerly** push the blob-stripped
  `ResourceCreate` over the relay (`inner.send_changes`) and upload the binary
  out-of-band (`upload_blob`). Rationale: the server accepts a blob only for
  already-materialised metadata, and the *periodic* relay cycle always loses
  that race for a brand-new resource. The periodic cycle may re-send the same
  change later; server materialisation is version-vector-idempotent, so the
  duplicate is harmless.
- `read_resource` — inner read; when the local cache has no bytes but
  `size > 0` (metadata synced from another device), fetch from the server
  (`download_blob`), falling back to the empty local data with a warning.
- `delete_resource`, `list_resources`, `purge_deleted_resources` —
  delegation.

**Dependencies** — the inner backend, `upload_blob`, `download_blob`,
`Change::ResourceCreate`.

**Used by** — resource traffic in server mode.

**Repeated context** — none.

---

## impl SyncBackend for CollabBackend

**Identification** — marker `// md:impl SyncBackend for CollabBackend`.

**What it does** — Delegation for `get_device_id` / `get_last_sync_time` /
`update_sync_time` / `send_changes` / `receive_changes` /
`prune_change_journal`, with the two note filters:

- `get_changes_since` — inner changes, **minus note changes**
  (`is_note_change`) and with resource blobs stripped (`strip_resource_blob`):
  the collab channel owns notes; blobs are uploaded out-of-band and must not
  bloat the relay journal.
- `apply_change` — a note change is dropped (`Ok(())`); everything else
  delegates.

**Dependencies** — `is_note_change`, `strip_resource_blob`, the inner backend.

**Used by** — the relay sync cycle in server mode.

**Repeated context** — this filter is the "notes travel exactly one path"
invariant.

---

## impl HistoryRepository for CollabBackend

**Identification** — marker `// md:impl HistoryRepository for CollabBackend`.

**What it does** — Pure delegation of `note_history`/`notebook_history`.

**Dependencies** — the inner backend.

**Used by** — history endpoints.

**Repeated context** — none.

---

## fn is_note_change

**Identification** — `fn is_note_change(change: &Change) -> bool`; marker
`// md:fn is_note_change`.

**What it does** — `true` for `NoteCreate`/`NoteUpdate`/`NoteDelete`.

**Dependencies** — `Change`.

**Used by** — the `SyncBackend` filters.

**Repeated context** — none.

---

## fn strip_resource_blob

**Identification** — `fn strip_resource_blob(change: Change) -> Change`; marker
`// md:fn strip_resource_blob`.

**What it does** — Drops the inline binary from a `ResourceCreate` before it
is relayed: with keeplin-srv the blob is uploaded out-of-band and served from
the server, so carrying it in the change would duplicate it and bloat the
journal.

**Dependencies** — `Change`.

**Used by** — `get_changes_since`.

**Repeated context** — none.

---

## ServerNote

**Identification** — private
`#[derive(Debug, serde::Deserialize)] struct ServerNote`; marker
`// md:ServerNote`.

**What it does** — The server's note representation from `GET /api/notes`:
`id`, `title`, `notebook_id: Option<Uuid>`, the todo fields, timestamps. No
body — bodies arrive via `Welcome` snapshots.

**Dependencies** — `serde`, `chrono`, `uuid`.

**Used by** — `discover_and_join`, `ensure_local`.

**Repeated context** — none.

---

## fn run_connection

**Identification** —
`async fn run_connection(shared: Arc<Shared>, out: mpsc::UnboundedReceiver<CollabClientMsg>)`;
marker `// md:fn run_connection`.

**What it does** — Maintains the WebSocket connection forever: `connect_once`
in a loop, resetting the backoff to 1 s after a clean run and doubling it up
to 30 s after failures. Every reconnect re-discovers and re-joins, so state is
rebuilt from snapshots.

**Dependencies** — `connect_once`, `tokio::time`.

**Used by** — spawned by `CollabBackend::start`.

**Repeated context** — none.

---

## REDISCOVER_EVERY

**Identification** —
`const REDISCOVER_EVERY: Duration = Duration::from_secs(15);` marker
`// md:REDISCOVER_EVERY`.

**What it does** — How often a live connection re-runs note discovery, so
notes created on other devices — or newly shared with this user — get joined
without a reconnect.

**Dependencies** — none.

**Used by** — `connect_once`.

**Repeated context** — none.

---

## fn connect_once

**Identification** —
`async fn connect_once(shared: &Arc<Shared>, out: &mut mpsc::UnboundedReceiver<CollabClientMsg>) -> anyhow::Result<()>`;
marker `// md:fn connect_once`.

**What it does** — One connection lifetime: build the WebSocket request with
the token in the **Authorization header, not the URL** (query strings end up
in proxy and access logs), connect, run `discover_and_join`, then a `select!`
loop over three arms:

- **outbound queue** — a queued `Join` for a note this connection already
  joined is dropped (the rediscovery got there first; a duplicate Join would
  fetch a second snapshot that may predate ops just sent, and its `Welcome`
  would clobber the mirror with that stale state); everything else is
  serialised and sent. A closed queue ends the task cleanly.
- **incoming frames** — text frames parsed as `CollabServerMsg` →
  `handle_server_msg`; a closed socket bails (triggering reconnect).
- **rediscovery tick** (every `REDISCOVER_EVERY`; the immediate first tick is
  consumed up front) — `discover_and_join` again.

**Dependencies** — `tokio_tungstenite`, `discover_and_join`,
`handle_server_msg`, `serde_json`.

**Used by** — `run_connection`.

**Repeated context** — none.

---

## DISCOVER_PAGE_SIZE

**Identification** — `const DISCOVER_PAGE_SIZE: &str = "200";` marker
`// md:DISCOVER_PAGE_SIZE`.

**What it does** — Page size for note discovery. The server caps `?limit` at
500 (keeplin-srv #29); a smaller page keeps each round-trip bounded on large
accounts.

**Dependencies** — none.

**Used by** — `discover_and_join`.

**Repeated context** — none.

---

## fn discover_and_join

**Identification** —
`async fn discover_and_join(shared, ws: &mut WsStream, joined: &mut HashSet<Uuid>) -> anyhow::Result<()>`;
marker `// md:fn discover_and_join`.

**What it does** — Discovers own + shared notes and joins the ones this
connection hasn't joined yet. Pages through `GET /api/notes?limit=&cursor=`,
following the `X-Next-Cursor` header until exhausted, so an account with
thousands of notes never materialises the whole listing at once;
back-compatible with a pre-pagination server (it ignores `?limit`, returns one
array with no cursor header — the loop runs exactly once). Unknown notes are
created locally via `ensure_local` (empty body — the `Welcome` snapshot fills
it in); each new note gets a `Join` sent directly on the socket and its id
recorded in `joined`.

**Dependencies** — `auth`, `reqwest`, `ServerNote`, `ensure_local`,
`CollabClientMsg::Join`.

**Used by** — `connect_once` (at connect and on every rediscovery tick).

**Repeated context** — none.

---

## WsStream

**Identification** —
`type WsStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>;`
marker `// md:WsStream`.

**What it does** — The client-side WebSocket stream type (plain TCP or TLS).

**Dependencies** — `tokio_tungstenite`.

**Used by** — `discover_and_join`'s signature.

**Repeated context** — none.

---

## fn ensure_local

**Identification** —
`async fn ensure_local(shared: &Arc<Shared>, server_note: &ServerNote)`; marker
`// md:fn ensure_local`.

**What it does** — Makes sure a note discovered on the server exists locally:
no-op when `top` is unset or the note already reads; otherwise builds a local
`Note` from the server metadata (empty body; a missing server notebook maps to
the nil UUID — locally the Inbox system notebook) and creates it through
`apply_from_server` (suppressed). Failures logged.

**Dependencies** — `apply_from_server`, `Note`.

**Used by** — `discover_and_join`.

**Repeated context** — none.

---

## fn handle_server_msg

**Identification** —
`async fn handle_server_msg(shared: &Arc<Shared>, msg: CollabServerMsg)`;
marker `// md:fn handle_server_msg`.

**What it does** — Dispatches one server message:

- `Welcome { note_id, snapshot }` — build the mirror from the snapshot. If the
  note was in `pending_push` (local content the server has not seen — it was
  just created, or edited before this first Welcome): **do not overwrite** —
  read the latest local body from `top`, diff it against the snapshot into
  ops, install the mirror, queue the ops; the local body stays as-is (already
  equal to what was diffed). Otherwise (a note we only cache): install the
  mirror and write the server's materialised body locally via `write_body`.
- `Op { note_id, ops, .. }` — apply each op to the mirror (unknown note:
  ignore), then `write_body` the new materialisation.
- `Error { code, message }` — log a warning.
- `Presence { note_id, users }` — replace that note's presence list.

**Dependencies** — `NoteLines`, `write_body`, the
`pending_push`/`notes`/`presence` state.

**Used by** — `connect_once`.

**Repeated context** — the pending-push reconcile is the "late empty Welcome
must not clobber local content" invariant, restated in `create_note`.

---

## fn write_body

**Identification** —
`async fn write_body(shared: &Arc<Shared>, note_id: Uuid, body: String)`;
marker `// md:fn write_body`.

**What it does** — Persists a server-derived body locally (suppressed),
keeping metadata: read the note from `top`; if it exists and the body differs,
set body + `updated_at = now` and `apply_from_server(update)`; if it does not
exist, create a fresh note with that id and body. Equal bodies are a no-op
(no version churn). Failures logged.

**Dependencies** — `apply_from_server`, `Note`.

**Used by** — `handle_server_msg` (`Welcome` cache branch and `Op`).

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Shared` — defined here (EXTRACTED; 85 cross-file edge(s))
- `CollabBackend<B>` — defined here (EXTRACTED; 6 cross-file edge(s))
- `CollabBackend` — defined here (EXTRACTED; 5 cross-file edge(s))
- `.note_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.notebook_history()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `.apply_from_server()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `CollabHandle` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.start()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.create_note()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `.read_note()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/protocol.rs` — collaborative channel wire types (EXTRACTED: references×8; e.g. `CollabClientMsg`, `PresenceInfo`, `Cursor`)
- `keeplin-core/src/collab/state.rs` — client line state and body↔lines translation (EXTRACTED: imports_from×1, references×1; e.g. `NoteLines`)
- `keeplin-core/src/error.rs` — error types (EXTRACTED: imports_from×1, references×41; e.g. `StorageError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×31; e.g. `Note`, `Notebook`, `Tag`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: implements×6, references×5; e.g. `StorageBackend`, `NotebookRepository`, `NoteRepository`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e) (EXTRACTED: references×2; e.g. `client()`, `wait_body()`)
- `keeplin-daemon/src/main.rs` — daemon entry point (EXTRACTED: imports_from×1, references×3; e.g. `collab_config()`, `run_server_with()`, `collab_starter()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×82; e.g. `add_link()`, `add_note_tag()`, `auth_mw()`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | module declarations + imports | `// md:Overview` |
| 2 | `struct CollabConfig` | `// md:CollabConfig` |
| 3 | `fn device_id_from_token` | `// md:fn device_id_from_token` |
| 4 | `struct Shared` | `// md:Shared` |
| 5 | `impl Shared` (+ `auth`, `suppressed`, `apply_from_server`, `upload_blob`, `download_blob`) | `// md:impl Shared` (+ `> fn …`) |
| 6 | `struct CollabHandle` | `// md:CollabHandle` |
| 7 | `impl CollabHandle` (+ `presence`, `send_cursor`, `proxy_request`) | `// md:impl CollabHandle` (+ `> fn …`) |
| 8 | `struct CollabBackend` | `// md:CollabBackend` |
| 9 | `impl Clone for CollabBackend` | `// md:impl Clone for CollabBackend` |
| 10 | `impl CollabBackend` (+ `new`, `handle`, `start`, `push_local_edit`, `patch_meta`) | `// md:impl CollabBackend` (+ `> fn …`) |
| 11 | `impl NoteRepository for CollabBackend` (9 methods) | `// md:impl NoteRepository for CollabBackend` |
| 12 | `impl NotebookRepository for CollabBackend` (5 methods) | `// md:impl NotebookRepository for CollabBackend` |
| 13 | `impl TagRepository for CollabBackend` (8 methods) | `// md:impl TagRepository for CollabBackend` |
| 14 | `impl ResourceRepository for CollabBackend` (5 methods) | `// md:impl ResourceRepository for CollabBackend` |
| 15 | `impl SyncBackend for CollabBackend` (8 methods) | `// md:impl SyncBackend for CollabBackend` |
| 16 | `impl HistoryRepository for CollabBackend` (2 methods) | `// md:impl HistoryRepository for CollabBackend` |
| 17 | `fn is_note_change` | `// md:fn is_note_change` |
| 18 | `fn strip_resource_blob` | `// md:fn strip_resource_blob` |
| 19 | `struct ServerNote` | `// md:ServerNote` |
| 20 | `fn run_connection` | `// md:fn run_connection` |
| 21 | `const REDISCOVER_EVERY` | `// md:REDISCOVER_EVERY` |
| 22 | `fn connect_once` | `// md:fn connect_once` |
| 23 | `const DISCOVER_PAGE_SIZE` | `// md:DISCOVER_PAGE_SIZE` |
| 24 | `fn discover_and_join` | `// md:fn discover_and_join` |
| 25 | `type WsStream` | `// md:WsStream` |
| 26 | `fn ensure_local` | `// md:fn ensure_local` |
| 27 | `fn handle_server_msg` | `// md:fn handle_server_msg` |
| 28 | `fn write_body` | `// md:fn write_body` |
