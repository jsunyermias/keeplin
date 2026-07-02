# `linking.rs` — `LinkingBackend` decorator + reference resolution

## Purpose

Turns the pure grammar in `links.rs` into behaviour. It provides:

1. **`LinkingBackend<B>`** — a `StorageBackend` decorator that, on every note write, derives
   the note's bookmarks and content links from its body, resolves each link to a target note,
   and enforces that note/notebook aliases are unique.
2. **Free helper functions** (usable through a type-erased `&dyn StorageBackend`) that the
   REST/gRPC surfaces call: reference resolution, backlinks, alias setters, manual-link
   add/remove, and alias-collision listing.

## Placement in the decorator stack

`LinkingBackend` sits **outside** any `EncryptedBackend` (so it parses the **plaintext** body
and resolves aliases against decrypted reads) and **inside** `EventBackend` (so the live feed
carries the refreshed metadata):

```
EventBackend( LinkingBackend( [EncryptedBackend]( Fs | Db ) ) )
```

## What it does on a note create/update

`prepare(note)` runs before delegating to the inner backend:

1. **Refresh (pure).** Rebuild `note.bookmarks` from the body's `[text](### "alias")`
   declarations (alias = title, else text; number = order). Rebuild content links from the
   body's `[t](#…)` markdown links, keeping any existing `Manual` links. The **body is the
   single source of truth** for bookmarks.
2. **Scan only if needed.** If the note has no alias and no links, skip the corpus scan
   entirely (the common case → O(1)). Otherwise fetch the note corpus (and notebooks only when
   there are links) to:
   - **Enforce alias uniqueness** — reject a create/update whose `alias` already belongs to
     another live note (`StorageError::Conflict`).
   - **Resolve links** — fill each link's `target_note_id` best-effort.

Reads, sync (`apply_change`), and the other entity types delegate unchanged.

## Concurrency

An `alias_write_lock` (`Mutex`) is held across the "scan for a duplicate → write" sequence,
but **only when the entity carries an alias**, so plain notes never serialise. This closes the
check-then-write race that could otherwise create a local duplicate alias.

## Free helper functions (called by the surfaces)

| Function | Purpose |
|----------|---------|
| `resolve(backend, raw)` | resolve a `#…` reference → `ResolvedReference { note_id, bookmark_number? }` |
| `backlinks(backend, target_id, page_size, page_token)` | paginated list of notes linking **to** a note (delegates to `note_backlinks`) |
| `set_note_alias` / `set_notebook_alias` | read-modify-write the alias (one `NoteUpdate`/`NotebookUpdate`); a soft-deleted target is `NotFound` (the edit must not revive it) |
| `add_manual_link` / `remove_link` | add a `Manual` link / remove a link by index; a soft-deleted note is `NotFound` |
| `alias_conflicts(backend)` | list aliases shared by 2+ live notes/notebooks (post-sync collisions) |
| `collect_notes` / `collect_notebooks` | exhaust the paginated `list_*` into a `Vec` |

## Resolution rules (`resolve_ref`, pure)

- `#note` → resolve the note (uuid as-is, else alias → smallest-uuid live match).
- `#a#b` → prefer `notebook#note`; if `b` is not a resolvable note, fall back to
  `note#bookmark`.
- `#notebook#note#bookmark` → resolve note (scoped by notebook), then map the bookmark
  alias/number to a stored number.

A duplicate alias (from a cross-device sync collision) resolves deterministically to the
smallest uuid, with a warning — surfaced for cleanup by `alias_conflicts`.

## Types

| Type | Meaning |
|------|---------|
| `LinkingBackend<B>` | the decorator |
| `ResolvedReference` | `{ note_id, bookmark_number: Option<u32> }` |
| `AliasConflict<T>` | `{ alias, entities: Vec<T> }` — one duplicated alias |
| `AliasConflicts` | `{ notes, notebooks }` — all current collisions |

## Design notes

- The corpus scan (`collect_notes`) re-materialises every note on `FsBackend`, so the
  "skip when nothing to check/resolve" short-circuit matters for write throughput.
- There is deliberately **no alias→uuid index**: alias resolution runs on decrypted values
  above the encryption boundary, and under encryption the stored alias is per-write ciphertext,
  so a database index could not answer an alias lookup.

## Related files

- `keeplin-core/src/links.rs` — the pure types/grammar this builds on.
- `keeplin-core/src/encryption.rs` — inner layer; must sit below `LinkingBackend`.
- `keeplin-daemon/src/{server,rest}.rs` — the surfaces that call the free helpers.
