# `collab/state.rs` — client line state and body↔lines translation

Self-contained companion for `keeplin-core/src/collab/state.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
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

**Code** — complete and verbatim:

```rust
// md:NoteLines
#[derive(Debug, Clone, Default)]
pub struct NoteLines {
    pub order: Vec<Uuid>,
    pub lines: HashMap<Uuid, LineSnapshot>,
    pub vv: VersionVector,
}
```

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

**Code** — container: members documented as sub-blocks below: fn from_snapshot, fn materialize, fn live, fn apply, fn diff_body.

**What it does** — Construction from a snapshot, body materialisation, the live-id
helper, server-op application, and the local-edit differ.

**Dependencies / Used by / Repeated context** — per method.

### fn from_snapshot

**Identification** — `pub fn from_snapshot(snapshot: NoteLinesSnapshot) -> Self`;
marker `// md:impl NoteLines > fn from_snapshot`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteLines > fn from_snapshot
    pub fn from_snapshot(snapshot: NoteLinesSnapshot) -> Self {
        Self {
            order: snapshot.order,
            lines: snapshot.lines.into_iter().map(|l| (l.id, l)).collect(),
            vv: snapshot.vv,
        }
    }
```

**What it does** — Builds the mirror from a `Welcome` snapshot: takes `order` and
the order `vv` verbatim, indexes `lines` by id into the map. Total; no validation —
the server is trusted.

**Dependencies** — `NoteLinesSnapshot`.

**Used by** — `collab/mod.rs` on every `Welcome` (connect and reconnect).

**Repeated context** — none.

### fn materialize

**Identification** — `pub fn materialize(&self) -> String`; marker
`// md:impl NoteLines > fn materialize`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteLines > fn materialize
    pub fn materialize(&self) -> String {
        self.order
            .iter()
            .filter_map(|id| self.lines.get(id))
            .filter(|l| l.deleted_at.is_none())
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl NoteLines > fn live
    fn live(&self) -> Vec<Uuid> {
        self.order
            .iter()
            .filter(|id| self.lines.get(id).is_some_and(|l| l.deleted_at.is_none()))
            .copied()
            .collect()
    }
```

**What it does** — Live line ids in order (the rows a body edit diffs against):
same filter as `materialize`, returning ids instead of content.

**Dependencies** — none.

**Used by** — `diff_body` (its only caller).

**Repeated context** — none.

### fn apply

**Identification** — `pub fn apply(&mut self, op: &LineOp)`; marker
`// md:impl NoteLines > fn apply`.

**Code** — complete and verbatim:

```rust
    // md:impl NoteLines > fn apply
    pub fn apply(&mut self, op: &LineOp) {
        match op {
            LineOp::Insert {
                after_line_id,
                line_id,
                content,
                vv,
                last_writer,
                updated_at,
            } => {
                let pos = match after_line_id {
                    None => 0,
                    Some(after) => self
                        .order
                        .iter()
                        .position(|id| id == after)
                        .map(|i| i + 1)
                        .unwrap_or(self.order.len()),
                };
                self.order.insert(pos, *line_id);
                self.lines.insert(
                    *line_id,
                    LineSnapshot {
                        id: *line_id,
                        content: content.clone(),
                        created_at: *updated_at,
                        updated_at: *updated_at,
                        deleted_at: None,
                        vv: vv.clone(),
                        last_writer: last_writer.clone(),
                    },
                );
                merge_into(&mut self.vv, vv);
            }
            LineOp::Update {
                line_id,
                content,
                vv,
                last_writer,
                updated_at,
            } => {
                if let Some(line) = self.lines.get_mut(line_id) {
                    line.content = content.clone();
                    line.updated_at = *updated_at;
                    line.deleted_at = None;
                    merge_into(&mut line.vv, vv);
                    line.last_writer = last_writer.clone();
                }
            }
            LineOp::Delete {
                line_id,
                deleted_at,
                vv,
                last_writer,
                updated_at,
            } => {
                if let Some(line) = self.lines.get_mut(line_id) {
                    line.deleted_at = Some(*deleted_at);
                    line.updated_at = *updated_at;
                    merge_into(&mut line.vv, vv);
                    line.last_writer = last_writer.clone();
                }
            }
            LineOp::Move {
                line_ids,
                after_line_id,
                vv,
                ..
            } => {
                self.order.retain(|id| !line_ids.contains(id));
                let pos = match after_line_id {
                    None => 0,
                    Some(after) => self
                        .order
                        .iter()
                        .position(|id| id == after)
                        .map(|i| i + 1)
                        .unwrap_or(self.order.len()),
                };
                self.order.splice(pos..pos, line_ids.iter().copied());
                merge_into(&mut self.vv, vv);
            }
        }
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl NoteLines > fn diff_body
    pub fn diff_body(&mut self, body: &str, device: &str) -> Vec<LineOp> {
        let now = Utc::now();
        let new_lines: Vec<&str> = if body.is_empty() {
            Vec::new()
        } else {
            body.split('\n').collect()
        };
        let old_ids = self.live();
        let old_contents: Vec<String> = old_ids
            .iter()
            .map(|id| self.lines[id].content.clone())
            .collect();

        let mut prefix = 0;
        while prefix < old_ids.len()
            && prefix < new_lines.len()
            && old_contents[prefix] == new_lines[prefix]
        {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < old_ids.len() - prefix
            && suffix < new_lines.len() - prefix
            && old_contents[old_ids.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
        {
            suffix += 1;
        }

        let old_mid = &old_ids[prefix..old_ids.len() - suffix];
        let new_mid: Vec<String> = new_lines[prefix..new_lines.len() - suffix]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut ops = Vec::new();
        let paired = old_mid.len().min(new_mid.len());

        for i in 0..paired {
            let id = old_mid[i];
            if self.lines[&id].content != new_mid[i] {
                let mut vv = self.lines[&id].vv.clone();
                bump(&mut vv, device);
                ops.push(LineOp::Update {
                    line_id: id,
                    content: new_mid[i].clone(),
                    vv,
                    last_writer: device.to_string(),
                    updated_at: now,
                });
            }
        }
        for id in &old_mid[paired..] {
            let mut vv = self.lines[id].vv.clone();
            bump(&mut vv, device);
            ops.push(LineOp::Delete {
                line_id: *id,
                deleted_at: now,
                vv,
                last_writer: device.to_string(),
                updated_at: now,
            });
        }
        let mut anchor = if paired > 0 {
            Some(old_mid[paired - 1])
        } else if prefix > 0 {
            Some(old_ids[prefix - 1])
        } else {
            None
        };
        let mut order_vv = self.vv.clone();
        for content in &new_mid[paired..] {
            bump(&mut order_vv, device);
            let vv = order_vv.clone();
            let line_id = Uuid::new_v4();
            ops.push(LineOp::Insert {
                after_line_id: anchor,
                line_id,
                content: content.clone(),
                vv,
                last_writer: device.to_string(),
                updated_at: now,
            });
            anchor = Some(line_id);
        }

        for op in &ops {
            self.apply(op);
        }
        ops
    }
```

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

**Code** — complete and verbatim:

```rust
// md:fn merge_into
fn merge_into(target: &mut VersionVector, other: &VersionVector) {
    for (k, v) in other {
        let entry = target.entry(k.clone()).or_insert(0);
        if *v > *entry {
            *entry = *v;
        }
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn bump
fn bump(vv: &mut VersionVector, device: &str) {
    *vv.entry(device.to_string()).or_insert(0) += 1;
}
```

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
