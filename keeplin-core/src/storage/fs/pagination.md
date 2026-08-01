# `storage/fs/pagination.rs` — cursor pagination helpers

Self-contained companion for `keeplin-core/src/storage/fs/pagination.rs`. It documents every source block in source order with complete code embedded for every leaf.

**How to navigate**: each source marker `// md:<Header> > …` maps to the matching header chain below.

---

## Overview

**Identification** — file-level module declarations and imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use uuid::Uuid;
```

**What it does** — Owns cursor pagination helpers. This is a structural relocation from the former monolithic filesystem module; storage behavior, on-disk format version 8, serialization shapes, conflict resolution, and public `storage::fs::FsBackend` API are unchanged.

**Dependencies** — every binding above is either a crate symbol used directly by the blocks below or a sibling item exposed as `pub(super)`; expects: those symbols to preserve the signatures and behavior the relocated bodies already relied on, with compile-time failure on drift.

**Used by** — sibling modules under `storage/fs/` and callers of `crate::storage::fs::FsBackend`.

**Repeated context** — `FsBackend::FORMAT_VERSION` remains 8. Rust makes `mod.rs` fields visible to descendant modules; sibling-defined helpers used across files carry only `pub(super)`.

---

## KeyedItem

**Identification** — private `struct KeyedItem<T>`; marker `// md:KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:KeyedItem
struct KeyedItem<T> {
    key: (String, Uuid),
    item: T,
}
```

**What it does** — An item tagged with its `(created_at_rfc3339, id)`
pagination key, ordered by the key alone so `PageCollector`'s max-heap can
evict the largest candidate.

---

## impl PartialEq for KeyedItem

**Identification** — marker `// md:impl PartialEq for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl PartialEq for KeyedItem
impl<T> PartialEq for KeyedItem<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
```

**What it does** — Key equality.

---

## impl Eq for KeyedItem

**Identification** — marker `// md:impl Eq for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl Eq for KeyedItem
impl<T> Eq for KeyedItem<T> {}
```

**What it does** — Marker impl.

---

## impl PartialOrd for KeyedItem

**Identification** — marker `// md:impl PartialOrd for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl PartialOrd for KeyedItem
impl<T> PartialOrd for KeyedItem<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
```

**What it does** — Delegates to `cmp`.

---

## impl Ord for KeyedItem

**Identification** — marker `// md:impl Ord for KeyedItem`.

**Code** — complete and verbatim:

```rust
// md:impl Ord for KeyedItem
impl<T> Ord for KeyedItem<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}
```

**What it does** — Key ordering.

---

## PageCollector

**Identification** — private `struct PageCollector<T>`; marker
`// md:PageCollector`.

**Code** — complete and verbatim:

```rust
// md:PageCollector
pub(super) struct PageCollector<T> {
    limit: usize,
    cursor: Option<(String, Uuid)>,
    heap: std::collections::BinaryHeap<KeyedItem<T>>,
}
```

**What it does** — Streaming replacement for collect-everything-then-paginate:
retains only the `limit + 1` smallest keys past the cursor in a max-heap, so
building one page holds O(page) items instead of the whole store; the `+1`
overflow slot is how it learns whether a next page exists. Cursor semantics
and the produced token match `paginate` exactly.

**Used by** — the note listing methods.

---

## impl PageCollector

**Identification** — inherent impl; marker `// md:impl PageCollector`. Three
methods.

**Code** — container: members documented as sub-blocks below: fn new, fn push, fn into_page.

---

### fn new

**Identification** — marker `// md:impl PageCollector > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCollector > fn new
    pub(super) fn new(limit: usize, token: Option<&str>) -> Self {
        let cursor = token
            .filter(|t| !t.is_empty())
            .and_then(|t| t.split_once('|'))
            .and_then(|(ts, id)| Uuid::parse_str(id).ok().map(|id| (ts.to_string(), id)));
        Self {
            limit,
            cursor,
            heap: std::collections::BinaryHeap::with_capacity(limit + 2),
        }
    }
```

**What it does** — Parses the `"<ts>|<uuid>"` cursor (`None`/empty/malformed
→ start at the beginning).

---

### fn push

**Identification** — marker `// md:impl PageCollector > fn push`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCollector > fn push
    pub(super) fn push(&mut self, key: (String, Uuid), item: T) {
        if let Some(cursor) = &self.cursor {
            if (key.0.as_str(), key.1) <= (cursor.0.as_str(), cursor.1) {
                return;
            }
        }
        if self.heap.len() <= self.limit {
            self.heap.push(KeyedItem { key, item });
        } else if let Some(top) = self.heap.peek() {
            if key < top.key {
                self.heap.pop();
                self.heap.push(KeyedItem { key, item });
            }
        }
    }
```

**What it does** — Offers one candidate: keys at or before the cursor are
skipped (the same predicate as `paginate`'s partition point); the rest compete
for the retained slots (heap eviction of the largest).

---

### fn into_page

**Identification** — marker `// md:impl PageCollector > fn into_page`.

**Code** — complete and verbatim:

```rust
    // md:impl PageCollector > fn into_page
    pub(super) fn into_page(self) -> (Vec<T>, Option<String>) {
        let mut items = self.heap.into_sorted_vec();
        let has_more = items.len() > self.limit;
        items.truncate(self.limit);
        let next_token = if has_more {
            items
                .last()
                .map(|last| format!("{}|{}", last.key.0, last.key.1))
        } else {
            None
        };
        (items.into_iter().map(|k| k.item).collect(), next_token)
    }
```

**What it does** — The retained items in ascending key order, trimmed to
`limit`, with a next-cursor when the overflow slot proved more exist.

---

## fn paginate

**Identification** —
`fn paginate<T, F>(items, limit, token, key_fn) -> (Vec<T>, Option<String>)`;
marker `// md:fn paginate`.

**Code** — complete and verbatim:

```rust
// md:fn paginate
pub(super) fn paginate<T, F>(
    items: Vec<T>,
    limit: usize,
    token: Option<&str>,
    key_fn: F,
) -> (Vec<T>, Option<String>)
where
    F: Fn(&T) -> (String, Uuid),
{
    let start = match token.filter(|t| !t.is_empty()) {
        Some(cursor) => {
            if let Some((ts, id_str)) = cursor.split_once('|') {
                if let Ok(cursor_id) = Uuid::parse_str(id_str) {
                    items.partition_point(|item| {
                        let (item_ts, item_id) = key_fn(item);
                        item_ts.as_str() < ts || (item_ts.as_str() == ts && item_id <= cursor_id)
                    })
                } else {
                    0
                }
            } else {
                0
            }
        }
        None => 0,
    };

    let remaining: Vec<T> = items.into_iter().skip(start).collect();
    let has_more = remaining.len() > limit;
    let page: Vec<T> = remaining.into_iter().take(limit).collect();

    let next_token = if has_more {
        page.last().map(|last| {
            let (ts, id) = key_fn(last);
            format!("{ts}|{id}")
        })
    } else {
        None
    };

    (page, next_token)
}
```

**What it does** — Cursor pagination over an already-sorted vec: partition
past the `"<ts>|<uuid>"` cursor (strictly after the cursor pair), take
`limit`, emit a next token from the page's last item when more remain.

**Used by** — the notebook/tag/resource listings (which sort small collected
vecs).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2; CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the same ignored `graphify-out/` layout locally.

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `cursor pagination helpers` — defined or implemented in this focused filesystem module (INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/storage/fs/mod.rs` and sibling `storage/fs/` modules — shared backend state and relocated helpers (INFERRED)
- `keeplin-core/src/storage/backend.rs`, `models.rs`, `error.rs`, and `storage/note_log.rs` as imported above — unchanged storage contracts (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- sibling `storage/fs/` modules and existing `FsBackend` callers — unchanged public module path (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- This split does not change the on-disk format; `FsBackend::FORMAT_VERSION` remains 8.
- The public backend path remains `crate::storage::fs::FsBackend`.
- Filesystem writes, journal replay, version-vector resolution, tombstones, and resource hashing preserve their pre-split behavior.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|

| 1 | `Overview` | `// md:Overview` |
| 2 | `KeyedItem` | `// md:KeyedItem` |
| 3 | `impl PartialEq for KeyedItem` | `// md:impl PartialEq for KeyedItem` |
| 4 | `impl Eq for KeyedItem` | `// md:impl Eq for KeyedItem` |
| 5 | `impl PartialOrd for KeyedItem` | `// md:impl PartialOrd for KeyedItem` |
| 6 | `impl Ord for KeyedItem` | `// md:impl Ord for KeyedItem` |
| 7 | `PageCollector` | `// md:PageCollector` |
| 8 | `impl PageCollector` (container) | `// md:impl PageCollector` |
| 9 | `fn new` | `// md:impl PageCollector > fn new` |
| 10 | `fn push` | `// md:impl PageCollector > fn push` |
| 11 | `fn into_page` | `// md:impl PageCollector > fn into_page` |
| 12 | `fn paginate` | `// md:fn paginate` |
