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

## Related files

- `keeplin-core/src/linking.rs` — the decorator + resolution that use this grammar.
- `keeplin-core/src/models.rs` — `Note` carries `bookmarks`/`links`; `Note`/`Notebook` carry `alias`.
- `README.md` — the user-facing "Bookmarks & links" section.
