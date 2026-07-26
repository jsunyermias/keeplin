# `storage/db/convert.rs` — JSON/version-vector/tombstone encoding helpers

Self-contained companion for `keeplin-core/src/storage/db/convert.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must
be able to understand it without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports the encoding helpers need. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::links::{Bookmark, NoteLink};

use crate::storage::note_log::VersionVector;
```

**What it does** — Pure, I/O-free encoding helpers shared across the directory module: cursor parsing and page building for keyset pagination, and the JSON encodings for bookmarks, links, version vectors, tombstones and associations. All are `pub(super)` because every sibling repository module calls them.

**Dependencies** — every binding above is either a crate this block's siblings call directly or a
path relocated from the pre-split `storage/db.rs`; expects: the symbols to keep the
signatures the block bodies below already rely on, since a changed signature fails to
compile rather than degrading silently.

**Used by** — the sibling modules of this directory module, and `crate::storage::db` through
`mod.rs`.

**Repeated context** — the directory module keeps `DbBackend`'s fields private in `mod.rs`;
Rust makes them visible to every descendant module, so siblings read them without any
widening. Items defined in one sibling and used by another carry `pub(super)`.

---

## fn parse_cursor

**Identification** — `fn parse_cursor(token: Option<&str>) -> (String, String)`;
marker `// md:fn parse_cursor`.

**Code** — complete and verbatim:

```rust
// md:fn parse_cursor
pub(super) fn parse_cursor(token: Option<&str>) -> (String, String) {
    match token.filter(|t| !t.is_empty()) {
        Some(cursor) => match cursor.split_once('|') {
            Some((ts, id)) => (ts.to_owned(), id.to_owned()),
            None => (String::new(), String::new()),
        },
        None => (String::new(), String::new()),
    }
}
```

**What it does** — Splits a `"<created_at>|<uuid>"` cursor into its parts;
absent/empty/malformed → `("", "")`, which makes the keyset SQL condition
`?1 = ''` match all rows (no offset).

**Used by** — every list method. **Repeated context** — none.

---

## fn build_page

**Identification** —
`fn build_page<T, F>(rows: Vec<T>, limit: usize, token_fn: F) -> (Vec<T>, Option<String>)`;
marker `// md:fn build_page`.

**Code** — complete and verbatim:

```rust
// md:fn build_page
pub(super) fn build_page<T, F>(
    mut rows: Vec<T>,
    limit: usize,
    token_fn: F,
) -> (Vec<T>, Option<String>)
where
    F: Fn(&T) -> String,
{
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    let next_token = if has_more {
        rows.last().map(token_fn)
    } else {
        None
    };
    (rows, next_token)
}
```

**What it does** — Turns a `LIMIT limit + 1` fetch into `(page, next_token)`:
more than `limit` rows ⇒ truncate and build the token from the page's last item;
otherwise no token.

**Used by** — every list method. **Repeated context** — none.

---

## fn bookmarks_to_json

**Identification** — marker `// md:fn bookmarks_to_json`.

**Code** — complete and verbatim:

```rust
// md:fn bookmarks_to_json
pub(super) fn bookmarks_to_json(bookmarks: &[Bookmark]) -> String {
    serde_json::to_string(bookmarks).unwrap_or_else(|_| "[]".to_string())
}
```

**What it does** — Serialises `notes.bookmarks` (`"[]"` fallback — a `Vec` of
small structs cannot fail in practice).

---

## fn links_to_json

**Identification** — marker `// md:fn links_to_json`.

**Code** — complete and verbatim:

```rust
// md:fn links_to_json
pub(super) fn links_to_json(links: &[NoteLink]) -> String {
    serde_json::to_string(links).unwrap_or_else(|_| "[]".to_string())
}
```

**What it does** — Serialises `notes.links` (`"[]"` fallback).

---

## fn json_to_bookmarks

**Identification** — marker `// md:fn json_to_bookmarks`.

**Code** — complete and verbatim:

```rust
// md:fn json_to_bookmarks
pub(super) fn json_to_bookmarks(s: &str) -> Vec<Bookmark> {
    serde_json::from_str(s).unwrap_or_default()
}
```

**What it does** — Parses the bookmarks column; malformed → empty list rather
than failing the read.

---

## fn json_to_links

**Identification** — marker `// md:fn json_to_links`.

**Code** — complete and verbatim:

```rust
// md:fn json_to_links
pub(super) fn json_to_links(s: &str) -> Vec<NoteLink> {
    serde_json::from_str(s).unwrap_or_default()
}
```

**What it does** — Parses the links column; malformed → empty list.

---

## fn vv_to_json

**Identification** — marker `// md:fn vv_to_json`.

**Code** — complete and verbatim:

```rust
// md:fn vv_to_json
pub(super) fn vv_to_json(vv: &VersionVector) -> String {
    serde_json::to_string(vv).unwrap_or_else(|_| "{}".to_string())
}
```

**What it does** — Serialises a version vector (`"{}"` fallback).

---

## fn json_to_vv

**Identification** — marker `// md:fn json_to_vv`.

**Code** — complete and verbatim:

```rust
// md:fn json_to_vv
pub(super) fn json_to_vv(s: &str) -> VersionVector {
    serde_json::from_str(s).unwrap_or_default()
}
```

**What it does** — Parses a `vv` column; malformed → empty vector (behaves as
an uninformed write).

---

## fn tombstone_data

**Identification** — marker `// md:fn tombstone_data`.

**Code** — complete and verbatim:

```rust
// md:fn tombstone_data
pub(super) fn tombstone_data(
    deleted_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> String {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}
```

**What it does** — Builds the journal `data` JSON for a delete:
`deleted_at` + the deleting write's `vv`/`last_writer`, so `row_to_change`
reconstructs a delete `Change` carrying everything `resolve` needs on the
receiving peer.

**Used by** — every delete path.

---

## fn assoc_data

**Identification** — marker `// md:fn assoc_data`.

**Code** — complete and verbatim:

```rust
// md:fn assoc_data
pub(super) fn assoc_data(
    tag_id: Uuid,
    updated_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> String {
    serde_json::json!({
        "tag_id": tag_id,
        "updated_at": updated_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}
```

**What it does** — Journal `data` JSON for a note↔tag add/remove: `tag_id` +
version metadata.

**Used by** — `add_note_tag`, `remove_note_tag`.

---

## fn assoc_from_data

**Identification** — marker `// md:fn assoc_from_data`.

**Code** — complete and verbatim:

```rust
// md:fn assoc_from_data
pub(super) fn assoc_from_data(
    data: &serde_json::Value,
    changed_at: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let updated_at = data
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(changed_at);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (updated_at, vv, last_writer)
}
```

**What it does** — Reconstructs `(updated_at, vv, last_writer)` from a journal
value, falling back to `changed_at` and empty vv/writer for pre-version
records.

**Used by** — `row_to_change`.

---

## fn tombstone_from_data

**Identification** — marker `// md:fn tombstone_from_data`.

**Code** — complete and verbatim:

```rust
// md:fn tombstone_from_data
pub(super) fn tombstone_from_data(
    data: &serde_json::Value,
    changed_at: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let deleted_at = data
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(changed_at);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (deleted_at, vv, last_writer)
}
```

**What it does** — Reconstructs `(deleted_at, vv, last_writer)` from a journal
value, same fallbacks.

**Used by** — `row_to_change`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `DbBackend` — extended here with the blocks below (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/db/mod.rs` — owns `DbBackend` and its fields (INFERRED)
- `keeplin-core/src/storage/backend.rs` — the repository traits and shared types (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/storage/db/mod.rs` — declares this submodule (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated
elsewhere)

- The split is a relocation: `storage::db::DbBackend` stays the public path, so no caller outside this directory module changes.
- These helpers stay I/O-free and total, so every caller can treat them as pure encoding.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | fn parse_cursor | `// md:fn parse_cursor` |
| 3 | fn build_page | `// md:fn build_page` |
| 4 | fn bookmarks_to_json | `// md:fn bookmarks_to_json` |
| 5 | fn links_to_json | `// md:fn links_to_json` |
| 6 | fn json_to_bookmarks | `// md:fn json_to_bookmarks` |
| 7 | fn json_to_links | `// md:fn json_to_links` |
| 8 | fn vv_to_json | `// md:fn vv_to_json` |
| 9 | fn json_to_vv | `// md:fn json_to_vv` |
| 10 | fn tombstone_data | `// md:fn tombstone_data` |
| 11 | fn assoc_data | `// md:fn assoc_data` |
| 12 | fn assoc_from_data | `// md:fn assoc_from_data` |
| 13 | fn tombstone_from_data | `// md:fn tombstone_from_data` |
