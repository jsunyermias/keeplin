# `collab/state.rs` — client line state and body↔lines translation

Self-contained companion for `keeplin-core/src/collab/state.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use super::protocol::{LineOp, LineSnapshot, NoteLinesSnapshot};
use crate::storage::note_log::VersionVector;
```

**What it does** — Client-side line state for one collaborative note, plus the
body↔lines translation: materialising the flat markdown body frontends see,
applying already-resolved server ops, and diffing a locally edited body into
`LineOp`s. **Pure logic, no I/O** — unit-testable in isolation. Two invariants the
whole collab feature rests on: applying the same ops in the same order on any
mirror materialises byte-identical bodies (deterministic replay), and diffing a
body then applying the resulting ops reproduces exactly that body.

**Dependencies** — `std::collections::HashMap`, `chrono` (op timestamps), `uuid`
(line ids), `collab/protocol.rs` wire types, `storage::note_log::VersionVector`
(the same version-vector type the rest of the project resolves with).

**Used by** — `collab/mod.rs` (calls `diff_body` on local edits,
`apply`/`from_snapshot` on inbound frames); `keeplin-core/tests/collab_client.rs`.

**Repeated context** — In the collab model a note is a set of independently
versioned **lines** plus a separately versioned **order** entity; conflict
resolution everywhere is version vectors first, then the deterministic
`(timestamp, device_id)` LWW tiebreak, and the vv actor is always the **device**
id, not the user id.

---

## NoteLines

**Identification** — struct deriving `Debug, Clone, Default`; marker
`// md:NoteLines`.

**What it does** — In-memory mirror of a note's server-side line entities:
`order: Vec<Uuid>` (the versioned sequence, live *and* tombstoned ids),
`lines: HashMap<Uuid, LineSnapshot>` (every line by id, tombstones included), and
`vv: VersionVector` (the **order** entity's vector). Nothing here is durable — the
mirror is rebuilt from the `Welcome` snapshot on every (re)connect; there is no
incremental catch-up.

**Dependencies** — `LineSnapshot`, `VersionVector`, `uuid`.

**Used by** — `collab/mod.rs` keeps one per joined note; `tests/collab_client.rs`.

**Repeated context** — Tombstones stay in the mirror (skipped only at
materialisation) so a later snapshot and this mirror converge on deletes; that is
the project's soft-delete-always premise applied to lines.

---

## impl NoteLines

**Identification** — the inherent impl block; marker `// md:impl NoteLines`. Five
methods, each with its own marker below.

**What it does** — Construction from a snapshot, body materialisation, the live-id
helper, server-op application, and the local-edit differ.

**Dependencies / Used by / Repeated context** — per method.

### fn from_snapshot

**Identification** — `pub fn from_snapshot(snapshot: NoteLinesSnapshot) -> Self`;
marker `// md:impl NoteLines > fn from_snapshot`.

**What it does** — Builds the mirror from a `Welcome` snapshot: takes `order` and
the order `vv` verbatim, indexes `lines` by id into the map. Total; no validation —
the server is trusted.

**Dependencies** — `NoteLinesSnapshot`.

**Used by** — `collab/mod.rs` on every `Welcome` (connect and reconnect).

**Repeated context** — none.

### fn materialize

**Identification** — `pub fn materialize(&self) -> String`; marker
`// md:impl NoteLines > fn materialize`.

**What it does** — The flat body frontends see: walk `order`, keep lines that
exist and have `deleted_at: None`, join their contents with `'\n'`. Ids present in
`order` but missing from `lines` are silently skipped. An empty mirror produces
`""`.

**Dependencies** — none beyond the struct.

**Used by** — `collab/mod.rs` when pushing the collaborative body into the local
store / to frontends.

**Repeated context** — `materialize` and `diff_body` must be inverses over live
content: `diff_body(materialize())` yields no ops.

### fn live

**Identification** — private `fn live(&self) -> Vec<Uuid>`; marker
`// md:impl NoteLines > fn live`.

**What it does** — Live line ids in order (the rows a body edit diffs against):
same filter as `materialize`, returning ids instead of content.

**Dependencies** — none.

**Used by** — `diff_body` (its only caller).

**Repeated context** — none.

### fn apply

**Identification** — `pub fn apply(&mut self, op: &LineOp)`; marker
`// md:impl NoteLines > fn apply`.

**What it does** — Applies one **already-resolved** server op to the mirror. The
server is the source of truth: ops arrive validated and in `server_seq` order, so
they are applied directly without re-resolving. Per variant:

- `Insert` — compute the position (`after_line_id: None` → head; anchor found →
  after it; anchor unknown → append), insert the id into `order`, build a fresh
  `LineSnapshot` (`created_at = updated_at = op.updated_at`, no tombstone), and
  merge the op's vv into the **order** vector.
- `Update` — if the line exists: replace content, set `updated_at`, clear any
  tombstone (`deleted_at = None` — an update revives a tombstoned line), merge the
  op's vv into the **line's** vector, set `last_writer`. Unknown id → no-op.
- `Delete` — if the line exists: set the tombstone and `updated_at`, merge vv,
  set `last_writer`. The id stays in `order`. Unknown id → no-op.
- `Move` — remove the moved ids from `order`, recompute the anchor position the
  same way as `Insert`, splice the ids back contiguously, merge vv into the
  **order** vector.

**Dependencies** — `merge_into`, `LineOp`, `LineSnapshot`.

**Used by** — `collab/mod.rs` for every inbound `CollabServerMsg::Op`; `diff_body`
(optimistic local echo).

**Repeated context** — line edits (`Update`/`Delete`) touch only that line's vv;
order edits (`Insert`/`Move`) touch only the order vv — the split that keeps
concurrent content edits and reorders from falsely conflicting.

### fn diff_body

**Identification** —
`pub fn diff_body(&mut self, body: &str, device: &str) -> Vec<LineOp>`; marker
`// md:impl NoteLines > fn diff_body`.

**What it does** — Diffs the current live lines against a newly edited flat
`body` and returns the ops that turn one into the other, applying them to the
mirror as they are generated (optimistic local echo). Algorithm:

1. Split `body` on `'\n'` (empty body → zero lines, so a cleared note tombstones
   everything).
2. Trim the common **prefix** and **suffix** of unchanged lines against `live()`.
3. Pair the changed middle **positionally**: differing pairs → `Update` (line vv
   cloned and `bump`ed for `device`); surplus old lines → `Delete` (tombstone);
   surplus new lines → `Insert` after the last paired/prefix line, each with a
   fresh `Uuid::new_v4()` id, chaining `anchor` so consecutive inserts land in
   sequence.
4. Apply every generated op to the mirror before returning, so the local view
   matches what was sent.

**The insert invariant** (the subtle part): inserts resolve against the *order*
entity, so several inserts in one edit must carry **strictly increasing** vectors.
The code clones the order vv once (`order_vv`) and `bump`s it per insert — N added
lines emit N ops whose order components strictly increase. Cloning `self.vv` fresh
per insert would give every op the same vector and the server would drop all but
the first as replays (a real historical bug — keeplin-srv PR #90).

Positional pairing, not full LCS: cheap and good enough for line editing; a moved
block shows up as deletes+inserts (the explicit `Move` op is for reorder UX, never
inferred here). All ops share one `Utc::now()` timestamp and `device` as
`last_writer`.

**Dependencies** — `live`, `bump`, `apply`, `chrono`, `uuid`.

**Used by** — `collab/mod.rs` on every local body edit;
`tests/collab_client.rs`.

**Repeated context** — the `device` argument is the vv actor — the device id,
matching the protocol's authorship rule (the server rejects ops whose
`last_writer` isn't the authenticated device: `bad_writer`).

---

## fn merge_into

**Identification** —
`fn merge_into(target: &mut VersionVector, other: &VersionVector)`; marker
`// md:fn merge_into`.

**What it does** — Element-wise max of two version vectors (least upper bound):
for each component in `other`, raise `target`'s entry if smaller, inserting
missing keys at that value. Never lowers a component.

**Dependencies** — `VersionVector` (a `BTreeMap/HashMap`-style `String → u64`
mapping — see `storage/note_log.rs`).

**Used by** — every branch of `apply`.

**Repeated context** — merging to the LUB is how a replica records "I have seen
at least this much history"; the same operation exists in `note_log`.

---

## fn bump

**Identification** — `fn bump(vv: &mut VersionVector, device: &str)`; marker
`// md:fn bump`.

**What it does** — Increments `device`'s component by one (inserting it at 1 if
absent) — the "I am about to write" step that makes a local op causally dominate
the state it was derived from.

**Dependencies** — `VersionVector`.

**Used by** — `diff_body` (per `Update`/`Delete`/`Insert` op).

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

- `NoteLines` — defined here (EXTRACTED; 4 cross-file edge(s))
- `.from_snapshot()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.apply()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.diff_body()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.materialize()` — defined here (EXTRACTED; file-local)
- `.live()` — defined here (EXTRACTED; file-local)
- `merge_into()` — defined here (EXTRACTED; file-local)
- `bump()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/protocol.rs` — collaborative channel wire types (EXTRACTED: references×4; e.g. `LineSnapshot`, `NoteLinesSnapshot`, `LineOp`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: imports_from×1, references×1; e.g. `mod.rs`, `Shared`)
- `keeplin-core/tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e) (EXTRACTED: imports_from×1; e.g. `collab_client.rs`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `struct NoteLines` | `// md:NoteLines` |
| 3 | `impl NoteLines` | `// md:impl NoteLines` |
| 4 | `fn from_snapshot` | `// md:impl NoteLines > fn from_snapshot` |
| 5 | `fn materialize` | `// md:impl NoteLines > fn materialize` |
| 6 | `fn live` | `// md:impl NoteLines > fn live` |
| 7 | `fn apply` | `// md:impl NoteLines > fn apply` |
| 8 | `fn diff_body` | `// md:impl NoteLines > fn diff_body` |
| 9 | `fn merge_into` | `// md:fn merge_into` |
| 10 | `fn bump` | `// md:fn bump` |
