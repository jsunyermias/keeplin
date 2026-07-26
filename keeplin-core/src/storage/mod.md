# `storage/mod.rs` — storage module root

Self-contained companion for `keeplin-core/src/storage/mod.rs`. It documents **every
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

**Identification** — file-level block: the child-module declarations and the
`backend` re-exports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
mod backend;
pub mod db;
pub mod fs;
pub mod note_log;

pub use backend::{
    EntityVersion, HistoryRepository, NoteRepository, NotebookRepository, NotebookSortProfile,
    ResourceRepository, StorageBackend, SyncBackend, TagRepository, DEFAULT_HISTORY_LIMIT,
};
```

**What it does** — The root of Keeplin's storage layer: the `StorageBackend` trait
family plus two concrete implementations. Module map:

| Module | Visibility | Role |
|--------|------------|------|
| `backend` | private (re-exported) | `StorageBackend` supertrait + its five sub-traits (`NoteRepository`, `NotebookRepository`, `TagRepository`, `ResourceRepository`, `SyncBackend`), `HistoryRepository`, `EntityVersion`, `NotebookSortProfile`, `DEFAULT_HISTORY_LIMIT` |
| `db` | public | `DbBackend` — local LibSQL (SQLite-compatible) database, synchronises with a central server over WebSocket |
| `fs` | public | `FsBackend` — JSON files on disk with per-device NDJSON change logs that Syncthing (or any compatible tool) replicates across devices |
| `note_log` | public | pure version-vector merge/resolution for the per-note logs (I/O-free, unit-tested); home of the LWW `(timestamp, device_id)` tiebreak used across the project |

`backend` is `mod` (not `pub mod`) so its private helpers stay off the public path;
its trait family is re-exported here, letting callers write
`keeplin_core::storage::StorageBackend`. `NotebookSortProfile` is the compact
per-notebook ordering summary (`pinned_keys`, `min_key`, `max_normal_key`) that the
`ordering` placement rules read; each backend builds it natively.

**Dependencies** — the four child modules.

**Used by** — every consumer of storage: the decorators (`encryption::EncryptedBackend`,
`linking::LinkingBackend`, `collab::CollabBackend`, daemon-side `EventBackend`/
`MetricsBackend`), `sync::engine`, `history`, `interop`, `migrate`, `ordering`, the
daemon (`server.rs`, `rest.rs`, `main.rs`), and the backend test suites.

**Repeated context** — Crate convention: new storage backends or decorators are added
as child modules implementing `StorageBackend`; nothing else here changes. No
re-exports at the *crate* root — `storage` is the shallowest public path.

---

## DEFAULT_PAGE_SIZE

**Identification** — `pub const DEFAULT_PAGE_SIZE: u32 = 100;` marker
`// md:DEFAULT_PAGE_SIZE`.

**Code** — complete and verbatim:

```rust
// md:DEFAULT_PAGE_SIZE
pub const DEFAULT_PAGE_SIZE: u32 = 100;
```

**What it does** — Page size used when a list call passes `page_size = 0` (the
"caller has no opinion" sentinel).

**Dependencies** — none.

**Used by** — `effective_page_size` below; documented contract of every backend list
method (`fs.rs`, `db.rs`) and of the daemon's list endpoints.

**Repeated context** — All list APIs in the project are cursor-paginated: reply
carries an opaque cursor; `page_size = 0` means "default".

---

## MAX_PAGE_SIZE

**Identification** — `pub const MAX_PAGE_SIZE: u32 = 1000;` marker
`// md:MAX_PAGE_SIZE`.

**Code** — complete and verbatim:

```rust
// md:MAX_PAGE_SIZE
pub const MAX_PAGE_SIZE: u32 = 1000;
```

**What it does** — Hard upper bound applied to every list call's `page_size`.
`page_size` arrives from the network (gRPC/REST) as an arbitrary `u32`; without a cap
a single request for `u32::MAX` rows would make the server materialise the entire
store in one response (memory-exhaustion DoS). Requests above the cap are **silently
clamped**, not rejected — the reply's cursor lets a well-behaved client keep paging.

**Dependencies** — none.

**Used by** — `effective_page_size` below.

**Repeated context** — clamp-don't-reject is the project's stance for out-of-range
paging inputs; domain-rule violations (e.g. pinning an inbox note) reject with
`StorageError::InvalidInput` instead.

---

## fn effective_page_size

**Identification** — `pub(crate) fn effective_page_size(page_size: u32) -> u32`;
marker `// md:fn effective_page_size`.

**Code** — complete and verbatim:

```rust
// md:fn effective_page_size
pub(crate) fn effective_page_size(page_size: u32) -> u32 {
    if page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        page_size.min(MAX_PAGE_SIZE)
    }
}
```

**What it does** — Resolves a caller-supplied `page_size` to the limit actually
used: `0` → `DEFAULT_PAGE_SIZE` (100); anything above `MAX_PAGE_SIZE` (1000) clamps
down to it; everything in between passes through. Pure, total, no errors.

**Dependencies** — the two constants above.

**Used by** — every list implementation in `storage/fs.rs` and `storage/db.rs`
(notes, notebooks, tags, resources, history listings).

**Repeated context** — crate-private (`pub(crate)`) on purpose: callers outside the
crate see only the *effect* (default + clamp), which the constants document.

---

## trait SortableRfc3339

**Identification** — `pub(crate) trait SortableRfc3339` with one method
`fn to_sortable_rfc3339(&self) -> String`; marker `// md:trait SortableRfc3339`.

**Code** — complete and verbatim:

```rust
// md:trait SortableRfc3339
pub(crate) trait SortableRfc3339 {
    fn to_sortable_rfc3339(&self) -> String;
}
```

**What it does** — Fixed-precision RFC 3339 formatting for timestamps that are
**compared as text**. The backends store timestamps as RFC 3339 TEXT and order them
lexicographically — SQLite `WHERE created_at > ?` / `ORDER BY`, and the `"<ts>|<id>"`
keyset cursors. Lexicographic order only matches chronological order when every value
has the same shape, but `DateTime::to_rfc3339()` emits a *variable* number of
fractional digits (3/6/9, whatever the instant needs — platform clock precision leaks
into the format). Two representations of comparable instants can then order
incorrectly, and the `created_at = cursor` equality branch of keyset pagination
silently fails across precisions. `to_sortable_rfc3339` pins the shape: always nine
fractional digits and the `+00:00` offset, so equal instants are equal strings and
lexicographic = chronological. Rows written before this existed keep their
variable-precision text; ordering against them stays chronologically consistent (the
shorter fraction sorts exactly where its value belongs) — only their cursor-equality
match remains best-effort, the same situation mixed-precision writers were already in.

**Dependencies** — none (trait definition only).

**Used by** — `storage/db.rs`, `storage/fs.rs`, and `storage/backend.rs` for every
stored/compared timestamp; the `impl` below provides the only implementation.

**Repeated context** — Timestamps-as-TEXT is a deliberate project convention (keeps
FS files human-readable and SQLite schema simple); this trait is the invariant that
makes it safe.

---

## impl SortableRfc3339 for DateTime Utc

**Identification** — the sole implementation, for `chrono::DateTime<chrono::Utc>`;
marker `// md:impl SortableRfc3339 for DateTime Utc`.

**Code** — complete and verbatim:

```rust
// md:impl SortableRfc3339 for DateTime Utc
impl SortableRfc3339 for chrono::DateTime<chrono::Utc> {
    fn to_sortable_rfc3339(&self) -> String {
        self.to_rfc3339_opts(chrono::SecondsFormat::Nanos, false)
    }
}
```

**What it does** — Delegates to
`self.to_rfc3339_opts(chrono::SecondsFormat::Nanos, false)`: `Nanos` forces exactly
nine fractional digits; `use_z: false` keeps the `+00:00` offset form (matching the
strings `to_rfc3339()` already produced, so old and new rows share the offset shape).

**Dependencies** — `chrono`.

**Used by** — call sites in `db.rs` / `fs.rs` / `backend.rs` via the trait.

**Repeated context** — none beyond the trait's.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`.
Three tests.

**Code** — container: members documented as sub-blocks below: fn effective_page_size_defaults_and_clamps, fn sortable_rfc3339_has_fixed_shape, fn lexicographic_order_matches_chronological_even_mixed_with_old_format.

**What it does** — Compile-time-gated unit tests for the two pure pieces of this
file (page-size clamping and timestamp shape). They run with `cargo test -p
keeplin-core` and never touch I/O.

**Dependencies** — `super::*` items, `chrono`.

**Used by** — CI (`cargo test --workspace`).

**Repeated context** — Project test convention: pure logic gets in-file
`#[cfg(test)]` unit tests; anything needing a running backend lives in
`keeplin-core/tests/`.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::SortableRfc3339;
    use chrono::{DateTime, TimeZone, Utc};
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::SortableRfc3339` — the exact item under test; expects: it stays public to the module tree and keeps its ordering semantics.
- `chrono::{DateTime, TimeZone, Utc}` — builds fixed UTC timestamps instead of reading the clock; expects: the `Utc` offset stays fixed and `with_ymd_and_hms` keeps resolving for these dates, so ordering assertions stay deterministic.

**Used by** — every block of `mod tests` in this file: `fn effective_page_size_defaults_and_clamps`, `fn sortable_rfc3339_has_fixed_shape`, `fn lexicographic_order_matches_chronological_even_mixed_with_old_format`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn effective_page_size_defaults_and_clamps

**Identification** — unit test; marker
`// md:mod tests > fn effective_page_size_defaults_and_clamps`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn effective_page_size_defaults_and_clamps
    #[test]
    fn effective_page_size_defaults_and_clamps() {
        assert_eq!(super::effective_page_size(0), super::DEFAULT_PAGE_SIZE);
        assert_eq!(super::effective_page_size(7), 7);
        assert_eq!(
            super::effective_page_size(super::MAX_PAGE_SIZE),
            super::MAX_PAGE_SIZE
        );
        assert_eq!(super::effective_page_size(u32::MAX), super::MAX_PAGE_SIZE);
    }
```

**What it does** — Asserts the three regimes of `effective_page_size`: `0` →
`DEFAULT_PAGE_SIZE`; in-range values (7, `MAX_PAGE_SIZE`) pass through;
`u32::MAX` clamps to `MAX_PAGE_SIZE`.

**Dependencies** — `effective_page_size`, the constants.

**Used by** — CI only.

**Repeated context** — none.

### fn sortable_rfc3339_has_fixed_shape

**Identification** — unit test; marker
`// md:mod tests > fn sortable_rfc3339_has_fixed_shape`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn sortable_rfc3339_has_fixed_shape
    #[test]
    fn sortable_rfc3339_has_fixed_shape() {
        let second_aligned = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let s = second_aligned.to_sortable_rfc3339();
        assert!(s.ends_with("+00:00"), "offset form is kept: {s}");
        let frac = s.split('.').nth(1).unwrap();
        assert_eq!(
            &frac[..9],
            "000000000",
            "always nine fractional digits: {s}"
        );
    }
```

**What it does** — Formats a second-aligned instant (worst case: `to_rfc3339()`
would emit *zero* fractional digits) and asserts the output ends with `+00:00` and
carries exactly nine fractional digits (`000000000`).

**Dependencies** — `SortableRfc3339`, `chrono`.

**Used by** — CI only.

**Repeated context** — none.

### fn lexicographic_order_matches_chronological_even_mixed_with_old_format

**Identification** — unit test; marker
`// md:mod tests > fn lexicographic_order_matches_chronological_even_mixed_with_old_format`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn lexicographic_order_matches_chronological_even_mixed_with_old_format
    #[test]
    fn lexicographic_order_matches_chronological_even_mixed_with_old_format() {
        let instants: Vec<DateTime<Utc>> = [
            (100, 0),
            (100, 500_000_000),
            (100, 500_000_001),
            (100, 999_999_999),
            (101, 0),
            (101, 123_456_000),
        ]
        .iter()
        .map(|&(s, n)| Utc.timestamp_opt(s, n).unwrap())
        .collect();

        let mut tagged: Vec<(DateTime<Utc>, String)> = Vec::new();
        for t in &instants {
            tagged.push((*t, t.to_rfc3339()));
            tagged.push((*t, t.to_sortable_rfc3339()));
        }
        let mut by_string = tagged.clone();
        by_string.sort_by(|a, b| a.1.cmp(&b.1));
        let mut by_time = tagged;
        by_time.sort_by_key(|(t, _)| *t);
        assert_eq!(
            by_string.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            by_time.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            "string order must never contradict time order"
        );
    }
```

**What it does** — Builds a set of instants straddling second and sub-second
boundaries, renders each in **both** the legacy variable-precision `to_rfc3339()`
format and the fixed `to_sortable_rfc3339()` format, sorts the combined list once by
string and once by instant, and asserts the two orders agree — proving stores mixing
old- and new-format rows still order chronologically.

**Dependencies** — `SortableRfc3339`, `chrono`.

**Used by** — CI only.

**Repeated context** — this is the invariant that lets old rows keep their text
untouched (project premise: clean breaks, no data migrations).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `effective_page_size()` — defined here (EXTRACTED; file-local)
- `SortableRfc3339` — defined here (EXTRACTED; file-local)
- `chrono::DateTime<chrono::Utc>` — defined here (EXTRACTED; file-local)
- `.to_sortable_rfc3339()` — defined here (EXTRACTED; file-local)
- `effective_page_size_defaults_and_clamps()` — defined here (EXTRACTED; file-local)
- `sortable_rfc3339_has_fixed_shape()` — defined here (EXTRACTED; file-local)
- `lexicographic_order_matches_chronological_even_mixed_with_old_format()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | module declarations + re-exports | `// md:Overview` |
| 2 | `DEFAULT_PAGE_SIZE` | `// md:DEFAULT_PAGE_SIZE` |
| 3 | `MAX_PAGE_SIZE` | `// md:MAX_PAGE_SIZE` |
| 4 | `fn effective_page_size` | `// md:fn effective_page_size` |
| 5 | `trait SortableRfc3339` | `// md:trait SortableRfc3339` |
| 6 | `impl SortableRfc3339 for DateTime<Utc>` | `// md:impl SortableRfc3339 for DateTime Utc` |
| 7 | `mod tests` | `// md:mod tests` |
| 8 | `imports` | `// md:mod tests > imports` |
| 9 | `fn effective_page_size_defaults_and_clamps` | `// md:mod tests > fn effective_page_size_defaults_and_clamps` |
| 10 | `fn sortable_rfc3339_has_fixed_shape` | `// md:mod tests > fn sortable_rfc3339_has_fixed_shape` |
| 11 | `fn lexicographic_order_matches_chronological_even_mixed_with_old_format` | `// md:mod tests > fn lexicographic_order_matches_chronological_even_mixed_with_old_format` |
