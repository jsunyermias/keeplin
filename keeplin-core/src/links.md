# `links.rs` — bookmark & link types and pure parsing

## Purpose

Defines the data types and the **pure, I/O-free grammar** for the two note-navigation
features: **bookmarks** and **inter-note links**. Everything here is side-effect-free so the
grammar can be unit-tested in isolation. Anything that needs store access (resolving an alias
to a concrete note) lives in `linking.rs`.

## The two features in one sentence each

- **Bookmark** — an in-note anchor declared in the body as a markdown link whose destination
  is exactly `###`: `[text](### "alias")`.
- **Link** — a connection from one note to another, either parsed from a markdown link in the
  body (`[t](#…)`) or added manually via the API.

## Persisted types (fields on `Note`)

| Type | Fields | Notes |
|------|--------|-------|
| `Bookmark` | `number: u32`, `text: String`, `alias: String` | `text` = link text, `alias` = link title (default = text), `number` = order of appearance |
| `NoteLink` | `source: LinkSource`, `raw: String`, `target_note_id: Option<Uuid>` | only `raw` + `source` + resolved uuid are stored; the parsed form is derived on demand |
| `LinkSource` | `Content` \| `Manual` | content-derived (from body) vs manually added |

Keeping `NoteLink` to a single string (`raw`) plus a plaintext UUID makes at-rest encryption
simple: only `raw` is encrypted; `target_note_id` stays plaintext like `notebook_id`.

## Parsed / grammar types

| Type | Meaning |
|------|---------|
| `Reference` | one note/notebook segment: `Id(Uuid)` or `Alias(String)` |
| `BookmarkRef` | the optional 3rd segment: `Number(u32)` or `Alias(String)` |
| `LinkTarget` | a fully parsed reference: `{ notebook?, note, bookmark? }` |
| `DerivedBookmark` | a bookmark parsed from the body: `{ text, alias: Option<String> }` |

## Reference grammar

A link destination is `#`-separated. `parse_link_ref` reads it structurally:

| Form | Meaning |
|------|---------|
| `#<note>` | note by **alias or uuid** |
| `#<notebook>#<note>` | notebook + note (each **alias or uuid**) |
| `#<notebook>#<note>#<bookmark>` | + bookmark by **alias or number** |

`parse_link_ref` reads a two-segment `#a#b` as `notebook#note`. Resolution in `linking.rs` is
smarter: it keeps that reading when `b` resolves to a note, otherwise falls back to
`note#bookmark` (so `#note3#anchor5` works without naming a notebook).

## Pure functions

| Function | Returns | What it does |
|----------|---------|--------------|
| `parse_link_ref(s)` | `Option<LinkTarget>` | parse a `#…` reference (1–3 non-empty segments) |
| `parse_bookmarks(body)` | `Vec<DerivedBookmark>` | find every `[text](### "alias")` declaration, in order |
| `parse_content_links(body)` | `Vec<String>` | find every markdown link whose destination starts with `#` (excluding the bookmark `###`) |
| `Reference::parse(seg)` / `BookmarkRef::parse(seg)` | the enum | uuid/number → typed, else alias |
| `NoteLink::from_raw(raw, source)` | `Option<NoteLink>` | validate + build a link |
| `NoteLink::target()` | `Option<LinkTarget>` | re-parse the stored `raw` on demand |

## Regexes (the exact rules)

- **Bookmark:** `\[([^\]]*)\]\(\s*###\s*(?:"([^"]*)")?\s*\)` — a markdown link whose
  destination is exactly `###`. Group 1 = text, group 2 = optional title (alias).
- **Content link:** `\[[^\]]*\]\(\s*(#[^)\s]+)\s*\)` — a markdown link whose destination
  starts with `#`; a destination equal to `###` is filtered out (it is a bookmark).

## Design notes

- `Bookmark` and `NoteLink` derive `Hash`/`Eq` so `Note` (which contains them) keeps its
  `Hash`/`Eq` derives.
- Because a bookmark's destination is exactly `###` and a link's is `#a…`, the two never
  collide: a bookmark is never mistaken for a link and vice-versa.

## Graph context

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
- `.parse()` — defined here (EXTRACTED; file-local)
- `LinkTarget` — defined here (EXTRACTED; file-local)
- `DerivedBookmark` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: references×2; e.g. `Note`)
- `keeplin-core/src/storage/db.rs` — DbBackend (LibSQL + WebSocket storage) (EXTRACTED: references×4; e.g. `bookmarks_to_json()`, `json_to_bookmarks()`, `json_to_links()`)
- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: calls×1, references×1; e.g. `list_links()`, `add_link()`)
- `keeplin-daemon/src/server.rs` — gRPC service implementation (EXTRACTED: references×1; e.g. `link_source_str()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Pure, I/O-free grammar — anything needing store access lives in `linking.rs`.
- The `#…` reference and `[t](###)` bookmark grammar is a compatibility surface: changing it changes how existing note bodies parse.

## Related files

- `keeplin-core/src/linking.rs` — the decorator + resolution that use this grammar.
- `keeplin-core/src/models.rs` — `Note` carries `bookmarks`/`links`; `Note`/`Notebook` carry `alias`.
- `README.md` — the user-facing "Bookmarks & links" section.
