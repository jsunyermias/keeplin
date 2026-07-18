# `links.rs` — bookmark & link types + the pure `#…` grammar

Self-contained companion for `keeplin-core/src/links.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

```rust
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
```

**What it does** — Note bookmarks and inter-note links: the persisted
`Bookmark`/`NoteLink` types that live as fields on `crate::models::Note`, the parsed
`LinkTarget` grammar, and the pure functions that extract bookmarks and content
links from a markdown body. **Intentionally I/O-free** so the grammar is
unit-testable in isolation; resolving an alias/uuid reference to a concrete note
(which needs store access) lives in `crate::linking::LinkingBackend`.

*Bookmarks*: an in-note anchor written as a markdown link whose destination is
exactly `###` (a link that goes nowhere) — `[text](### "alias")`. The link **text**
becomes the bookmark's `text`; the optional link **title** becomes its `alias`
(defaulting to the text); its `number` is its 1-based position among the note's
bookmarks. The body is the single source of truth — there is no bookmark API;
bookmarks are created/renamed/removed by editing the body.

*Links*: standard markdown links whose destination starts with `#`, e.g.
`[text](#notebook1#note3#5)`, with grammar `#<note>` | `#<notebook>#<note>` |
`#<notebook>#<note>#<bookmark>` (each segment an alias or uuid; bookmark also by
1-based number). `parse_link_ref` is purely structural and reads two segments as
`notebook#note`; resolution in `crate::linking` keeps that reading when the middle
segment resolves as a note, otherwise falls back to `note#bookmark`, so
`#note3#anchor5` targets a bookmark without naming a notebook.

**Dependencies** — `regex` (+ `std::sync::OnceLock` for compile-once statics),
`serde`, `uuid`.

**Used by** — `models.rs` (`Note.bookmarks`/`Note.links`), `linking.rs` (derives
and resolves), `storage/db.rs` (JSON (de)serialisation helpers),
`keeplin-daemon/src/rest.rs` and `server.rs` (link endpoints).

**Repeated context** — The `#…` reference and `[t](###)` bookmark grammar is a
compatibility surface: changing it changes how existing note bodies parse. Because
a bookmark's destination is exactly `###` and a link's is `#a…`, the two can never
collide.

---

## Reference

**Identification** — enum deriving `Debug, Clone, PartialEq, Eq, Hash, Serialize,
Deserialize` with `#[serde(rename_all = "snake_case")]`; marker `// md:Reference`.

**What it does** — One note-or-notebook reference segment: `Id(Uuid)` (the segment
parsed as a valid UUID) or `Alias(String)` (anything else). Aliases are the
human-assigned names enforced unique per entity type by `LinkingBackend`
(duplicates rejected with `StorageError::Conflict`).

**Dependencies** — `uuid`, `serde`.

**Used by** — `LinkTarget.notebook`/`.note`; `linking.rs` resolution.

**Repeated context** — snake_case serde naming is the project's JSON convention
for enums.

---

## impl Reference

**Identification** — inherent impl; marker `// md:impl Reference`. One method.

**What it does** — Parsing for a single segment.

### fn parse

**Identification** — `pub fn parse(segment: &str) -> Self`; marker
`// md:impl Reference > fn parse`.

**What it does** — A valid UUID becomes `Reference::Id`, anything else
`Reference::Alias`. Total — never fails.

**Dependencies** — `Uuid::parse_str`.

**Used by** — `parse_link_ref` (every notebook/note segment).

**Repeated context** — none.

---

## BookmarkRef

**Identification** — enum deriving `Debug, Clone, PartialEq, Eq, Hash, Serialize,
Deserialize` with `#[serde(rename_all = "snake_case")]`; marker
`// md:BookmarkRef`.

**What it does** — The optional third segment of a link: `Number(u32)` (a bookmark
by its 1-based position in the note) or `Alias(String)` (by its default or edited
alias).

**Dependencies** — `serde`.

**Used by** — `LinkTarget.bookmark`; `linking.rs` bookmark resolution.

**Repeated context** — none.

---

## impl BookmarkRef

**Identification** — inherent impl; marker `// md:impl BookmarkRef`. One method.

**What it does** — Parsing for the bookmark segment.

### fn parse

**Identification** — `pub fn parse(segment: &str) -> Self`; marker
`// md:impl BookmarkRef > fn parse`.

**What it does** — An unsigned integer ≥ 1 becomes `Number`; anything else —
including `"0"`, because bookmark numbering is 1-based — an `Alias`. Total.

**Dependencies** — `str::parse::<u32>`.

**Used by** — `parse_link_ref` (three-segment form); unit test
`bookmark_ref_zero_is_alias`.

**Repeated context** — none.

---

## LinkTarget

**Identification** — struct deriving `Debug, Clone, PartialEq, Eq, Serialize,
Deserialize`; marker `// md:LinkTarget`.

**What it does** — A fully parsed `#…` reference: `notebook: Option<Reference>`
(present with two or more segments), `note: Reference` (always present),
`bookmark: Option<BookmarkRef>` (present with three). The optional fields carry
`#[serde(default, skip_serializing_if = "Option::is_none")]` so serialised targets
stay minimal.

**Dependencies** — `Reference`, `BookmarkRef`, `serde`.

**Used by** — returned by `parse_link_ref`/`NoteLink::target`; consumed by
`linking.rs` resolution and the daemon's link endpoints.

**Repeated context** — the two-segment structural reading is `notebook#note`;
`note#bookmark` is a *resolution-time* fallback in `linking.rs`, not encoded here.

---

## LinkSource

**Identification** — enum deriving `Debug, Clone, Copy, PartialEq, Eq, Hash,
Serialize, Deserialize` with `#[serde(rename_all = "snake_case")]`; marker
`// md:LinkSource`.

**What it does** — Where a `NoteLink` came from: `Content` (derived from a
markdown link in the body; recomputed on every write, so deleting the markdown
deletes the link) or `Manual` (added directly via the API; not present in the body
and preserved across body edits).

**Dependencies** — `serde`.

**Used by** — `NoteLink.source`; `linking.rs` (recomputes `Content`, preserves
`Manual`); `keeplin-daemon/src/server.rs::link_source_str`.

**Repeated context** — none.

---

## Bookmark

**Identification** — struct deriving `Debug, Clone, PartialEq, Eq, Hash,
Serialize, Deserialize`; marker `// md:Bookmark`.

**What it does** — A bookmark anchor within a note, as persisted on
`Note.bookmarks`: `number` (1-based position by order of appearance in the body),
`text` (the link text of the `[text](###)` declaration), `alias` (the link title,
defaulting to `text`). Derives `Hash`/`Eq` so `Note` — which contains a
`Vec<Bookmark>` — keeps its own `Hash`/`Eq` derives.

**Dependencies** — `serde`.

**Used by** — `models::Note.bookmarks`; built by `linking.rs` from
`parse_bookmarks` output; `storage/db.rs` JSON helpers.

**Repeated context** — bookmarks are body-derived: the persisted list is a cache
recomputed on every note write, never edited directly.

---

## DerivedBookmark

**Identification** — struct deriving `Debug, Clone, PartialEq, Eq` (not serde —
never persisted); marker `// md:DerivedBookmark`.

**What it does** — A bookmark as parsed from a body: `text` plus the optional
inline `alias` (the link title). The caller assigns the 1-based `number` by order
of appearance and defaults the alias to `text` when `None` — that's the step that
turns a `DerivedBookmark` into a persisted `Bookmark`.

**Dependencies** — none.

**Used by** — returned by `parse_bookmarks`; consumed by `linking.rs`.

**Repeated context** — none.

---

## NoteLink

**Identification** — struct deriving `Debug, Clone, PartialEq, Eq, Hash,
Serialize, Deserialize`; marker `// md:NoteLink`.

**What it does** — A link from one note to a target note (optionally scoped by
notebook and bookmark), as persisted on `Note.links`: `source` (`Content`/
`Manual`), `raw` (the literal `#…` string exactly as parsed or supplied), and
`target_note_id: Option<Uuid>` (best-effort resolution snapshot at write time;
`None` when the target didn't resolve, e.g. it did not exist yet). Only these
three are persisted — the parsed `LinkTarget` is derived on demand from `raw`.
Keeping the persisted form to one human string plus a UUID makes at-rest
encryption straightforward: only `raw` needs encrypting; the UUID stays plaintext,
like `notebook_id`.

**Dependencies** — `LinkSource`, `uuid`, `serde`.

**Used by** — `models::Note.links`; `linking.rs` (derivation, resolution,
backlinks); `storage/db.rs` (`note_links` projection); the daemon's link
endpoints.

**Repeated context** — at-rest encryption (project-wide) encrypts human-readable
content but leaves entity UUIDs plaintext so indexes and joins keep working.

---

## impl NoteLink

**Identification** — inherent impl; marker `// md:impl NoteLink`. Two methods.

**What it does** — Validated construction and on-demand parsing.

### fn from_raw

**Identification** —
`pub fn from_raw(raw: &str, source: LinkSource) -> Option<Self>`; marker
`// md:impl NoteLink > fn from_raw`.

**What it does** — Validates `raw` with `parse_link_ref` (returns `None` for an
invalid reference) and builds a `NoteLink` with `target_note_id: None` —
resolution happens later in `linking.rs`.

**Dependencies** — `parse_link_ref`.

**Used by** — `linking.rs` (content-derived links and the manual-add API path).

**Repeated context** — none.

### fn target

**Identification** — `pub fn target(&self) -> Option<LinkTarget>`; marker
`// md:impl NoteLink > fn target`.

**What it does** — Re-parses the persisted `raw` into its `LinkTarget`
components. `None` only if `raw` was corrupted after construction (construction
validates).

**Dependencies** — `parse_link_ref`.

**Used by** — `linking.rs` resolution; the daemon when rendering link targets.

**Repeated context** — none.

---

## fn parse_link_ref

**Identification** — `pub fn parse_link_ref(s: &str) -> Option<LinkTarget>`;
marker `// md:fn parse_link_ref`.

**What it does** — Parses a `#…` reference string: strips the leading `#`
(`None` without it), splits the rest on `#`, rejects any empty segment, and maps
one segment → note only, two → notebook + note, three → notebook + note +
bookmark; more than three → `None`. Each segment goes through
`Reference::parse`/`BookmarkRef::parse`, so the function is total over its
accepted shapes.

**Dependencies** — `Reference::parse`, `BookmarkRef::parse`.

**Used by** — `NoteLink::from_raw`/`target`; `linking.rs`; unit tests
`parses_one_two_three_segments`, `parses_uuid_segments_as_ids`,
`rejects_malformed_refs`.

**Repeated context** — purely structural; the `note#bookmark` fallback for
two-segment refs is applied at resolution time in `linking.rs`, never here.

---

## fn bookmark_re

**Identification** — `fn bookmark_re() -> &'static Regex` over a
`static OnceLock<Regex>`; marker `// md:fn bookmark_re`.

**What it does** — The compiled bookmark-declaration regex:
`\[([^\]]*)\]\(\s*###\s*(?:"([^"]*)")?\s*\)` — a markdown link whose destination
is exactly `###`, group 1 = link text, group 2 = optional quoted title (the
alias). Compiled once per process via `OnceLock`.

**Dependencies** — `regex`, `OnceLock`.

**Used by** — `parse_bookmarks`.

**Repeated context** — a markdown `### heading` does not match (no `[text](…)`
around it) — see test `extracts_bookmarks_with_and_without_alias_in_order`.

---

## fn content_link_re

**Identification** — `fn content_link_re() -> &'static Regex` over a
`static OnceLock<Regex>`; marker `// md:fn content_link_re`.

**What it does** — The compiled content-link regex:
`\[[^\]]*\]\(\s*(#[^)\s]+)\s*\)` — a markdown link whose destination starts with
`#` (at least one non-space, non-`)` char after it), capturing the destination.
Compiled once per process.

**Dependencies** — `regex`, `OnceLock`.

**Used by** — `parse_content_links`.

**Repeated context** — none.

---

## fn parse_bookmarks

**Identification** — `pub fn parse_bookmarks(body: &str) -> Vec<DerivedBookmark>`;
marker `// md:fn parse_bookmarks`.

**What it does** — Extracts every `[text](### "alias")` bookmark declaration in
`body`, in order of appearance. The 1-based number of each bookmark is its index
in the returned vector plus one (assigned by the caller). Duplicate texts are kept
— each occurrence is a distinct bookmark.

**Dependencies** — `bookmark_re`, `DerivedBookmark`.

**Used by** — `linking.rs` on every note write; test
`extracts_bookmarks_with_and_without_alias_in_order`.

**Repeated context** — the body is the single source of truth for bookmarks;
there is no bookmark CRUD API.

---

## fn parse_content_links

**Identification** — `pub fn parse_content_links(body: &str) -> Vec<String>`;
marker `// md:fn parse_content_links`.

**What it does** — Extracts the raw `#…` destinations of every markdown link in
`body`, in order of appearance. Non-`#` destinations never match the regex; a
destination equal to `###` is filtered out because it is a bookmark declaration,
not a link.

**Dependencies** — `content_link_re`.

**Used by** — `linking.rs` on every note write (derives `Content` links); test
`extracts_content_links_excluding_bookmarks`.

**Repeated context** — none.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`.
Six tests, all pure.

**What it does** — Pins the grammar: segment shapes, uuid detection, malformed
rejections, the 1-based-number rule, and both extraction functions.

**Dependencies** — `super::*`, `uuid`.

**Used by** — CI (`cargo test --workspace`).

**Repeated context** — the grammar is a compatibility surface, so these tests are
the contract — change them only with a deliberate format break.

### fn parses_one_two_three_segments

**Identification** — unit test; marker
`// md:mod tests > fn parses_one_two_three_segments`.

**What it does** — `#note3` → note-only alias; `#notebook1#note3` → notebook +
note; `#notebook1#note3#anchor5` → bookmark alias; `#notebook1#note3#5` →
bookmark `Number(5)`.

### fn parses_uuid_segments_as_ids

**Identification** — unit test; marker
`// md:mod tests > fn parses_uuid_segments_as_ids`.

**What it does** — Fresh UUID segments parse as `Reference::Id` for both notebook
and note positions.

### fn rejects_malformed_refs

**Identification** — unit test; marker
`// md:mod tests > fn rejects_malformed_refs`.

**What it does** — Rejects a missing leading `#` (`note3`), an empty single
segment (`#`), an empty middle segment (`#a##b`), and four segments (`#a#b#c#d`).

### fn bookmark_ref_zero_is_alias

**Identification** — unit test; marker
`// md:mod tests > fn bookmark_ref_zero_is_alias`.

**What it does** — `"0"` parses as `Alias("0")` (numbering is 1-based); `"1"` as
`Number(1)`.

### fn extracts_bookmarks_with_and_without_alias_in_order

**Identification** — unit test; marker
`// md:mod tests > fn extracts_bookmarks_with_and_without_alias_in_order`.

**What it does** — A body with `[Bookmark1](###)`, a `### heading` line (not a
bookmark), and `[Other](### "Alias")` yields exactly the two declarations, in
order, with `alias: None` and `alias: Some("Alias")` respectively.

### fn extracts_content_links_excluding_bookmarks

**Identification** — unit test; marker
`// md:mod tests > fn extracts_content_links_excluding_bookmarks`.

**What it does** — From a body with `[a](#note3)`, `[b](#notebook1#note3#5)`, a
bookmark `[c](###)`, an http link `[d](http://x)`, and `[e](#)` (matches nothing —
needs ≥ 1 char after `#`), only the two `#…` destinations are returned.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `NoteLink` — defined here (EXTRACTED; 4 cross-file edge(s))
- `Bookmark` — defined here (EXTRACTED; 3 cross-file edge(s))
- `LinkSource` — defined here (EXTRACTED; 1 cross-file edge(s))
- `parse_link_ref()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `Reference` — defined here (EXTRACTED; file-local)
- `.parse()` — defined here (EXTRACTED; file-local)
- `BookmarkRef` — defined here (EXTRACTED; file-local)
- `LinkTarget` — defined here (EXTRACTED; file-local)
- `DerivedBookmark` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×2; e.g. `Note`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×4; e.g. `bookmarks_to_json()`, `json_to_bookmarks()`, `json_to_links()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: calls×1, references×1; e.g. `list_links()`, `add_link()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×1; e.g. `link_source_str()`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `enum Reference` | `// md:Reference` |
| 3 | `impl Reference` | `// md:impl Reference` |
| 4 | `fn parse` (Reference) | `// md:impl Reference > fn parse` |
| 5 | `enum BookmarkRef` | `// md:BookmarkRef` |
| 6 | `impl BookmarkRef` | `// md:impl BookmarkRef` |
| 7 | `fn parse` (BookmarkRef) | `// md:impl BookmarkRef > fn parse` |
| 8 | `struct LinkTarget` | `// md:LinkTarget` |
| 9 | `enum LinkSource` | `// md:LinkSource` |
| 10 | `struct Bookmark` | `// md:Bookmark` |
| 11 | `struct DerivedBookmark` | `// md:DerivedBookmark` |
| 12 | `struct NoteLink` | `// md:NoteLink` |
| 13 | `impl NoteLink` | `// md:impl NoteLink` |
| 14 | `fn from_raw` | `// md:impl NoteLink > fn from_raw` |
| 15 | `fn target` | `// md:impl NoteLink > fn target` |
| 16 | `fn parse_link_ref` | `// md:fn parse_link_ref` |
| 17 | `fn bookmark_re` | `// md:fn bookmark_re` |
| 18 | `fn content_link_re` | `// md:fn content_link_re` |
| 19 | `fn parse_bookmarks` | `// md:fn parse_bookmarks` |
| 20 | `fn parse_content_links` | `// md:fn parse_content_links` |
| 21 | `mod tests` | `// md:mod tests` |
| 22 | `fn parses_one_two_three_segments` | `// md:mod tests > fn parses_one_two_three_segments` |
| 23 | `fn parses_uuid_segments_as_ids` | `// md:mod tests > fn parses_uuid_segments_as_ids` |
| 24 | `fn rejects_malformed_refs` | `// md:mod tests > fn rejects_malformed_refs` |
| 25 | `fn bookmark_ref_zero_is_alias` | `// md:mod tests > fn bookmark_ref_zero_is_alias` |
| 26 | `fn extracts_bookmarks_with_and_without_alias_in_order` | `// md:mod tests > fn extracts_bookmarks_with_and_without_alias_in_order` |
| 27 | `fn extracts_content_links_excluding_bookmarks` | `// md:mod tests > fn extracts_content_links_excluding_bookmarks` |
