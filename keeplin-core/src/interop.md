# `interop.rs` — vCard & iCalendar format compatibility

## Purpose

Makes Keeplin **compatible** with the vCard (RFC 6350) and iCalendar VTODO/VEVENT (RFC 5545)
formats — parsing them in and serialising them back out — so contacts and calendar items move
losslessly between Keeplin and other apps. Keeplin is deliberately **not** a WebDAV/CalDAV
server; this is interop at the data-format level only.

This module is **pure** (no I/O, no storage): the storage backends and the daemon build the
native entities and endpoints on top of these types (later stages of the front).

## Types

| Type | Maps to | Notes |
|------|---------|-------|
| `Contact` | a vCard 4.0 card | `FN`, `N` (family/given), `EMAIL*`, `TEL*`, `ORG`, `NOTE`, `UID` |
| `CalendarEvent` | a `VEVENT` | `SUMMARY`, `DTSTART`, `DTEND`, `LOCATION`, `DESCRIPTION`, `UID` |
| `CalendarTodo` | a `VTODO` | `SUMMARY`, `DUE`, `COMPLETED`/`STATUS`, `DESCRIPTION`, `UID` |

Each carries an `extra: Vec<String>` of properties the module does not model, preserved
verbatim so a parse→serialise round-trip drops nothing.

## API

| Function | Description |
|----------|-------------|
| `Contact::to_vcard()` / `from_vcard(&str)` | serialise / parse the first `VCARD` |
| `CalendarEvent::to_ics()` / `from_ics(&str)` | serialise / parse the first `VEVENT` (wrapped in a `VCALENDAR`) |
| `CalendarTodo::to_ics()` / `from_ics(&str)` | serialise / parse the first `VTODO` |
| `CalendarTodo::from_note(&Note)` / `apply_to_note(&mut Note)` | map a `VTODO` ⇄ a Keeplin to-do note (`title`↔`SUMMARY`, `body`↔`DESCRIPTION`, `todo_due`↔`DUE`, `todo_completed`↔`COMPLETED`; note id ↔ `UID`) |
| `user_vcard(display_name, email)` | render a profile card for an account owner |

## Format handling

- **Line folding** (`fold_line`/`unfold`): output is folded at 75 octets with `CRLF␣`; input is
  unfolded before parsing, so wrapped values rejoin.
- **TEXT escaping** (`escape_text`/`unescape_text`): `\`, `,`, `;` and newlines round-trip.
- **Date-times** are UTC (`YYYYMMDDTHHMMSSZ`). The trailing `Z` is a literal (not a `%z`
  offset), so parsing reads the naive wall-clock and stamps it UTC; a bare `YYYYMMDD` date is
  read as midnight UTC.
- **Property lines** are `NAME(;PARAM=…)*:VALUE`; parameters are dropped for the modelled
  subset (the raw line is kept in `extra` for anything unmodelled).

## Typed storage over resources

Contacts and events are **typed at the API level** but persist on top of the existing
**resource** entity — no new entity type, table, protobuf message, or sync `Change`, so they
ride the sync/encryption/permissions/server-materialisation machinery already built. A contact
is a resource with mime `text/vcard`; an event, `text/calendar`. **Identity is the format `UID`**
(not the backing resource id), and since resources have no in-place update, `save_*` is a
*replace by UID* (delete any resource with that UID, write a fresh one).

| Function | Description |
|----------|-------------|
| `save_contact(be, Contact)` / `save_event(be, CalendarEvent)` | upsert by UID (assigns one if empty); returns the stored value |
| `list_contacts(be)` / `list_events(be)` | parse every backing resource of the right mime type |
| `get_contact(be, uid)` / `get_event(be, uid)` | fetch one by UID |
| `delete_contact(be, uid)` / `delete_event(be, uid)` | remove the backing resource(s) |
| `import_todo(be, ics)` | parse a `VTODO` and create a Keeplin **to-do note** (to-dos are notes, not resources) |

`MIME_VCARD` / `MIME_ICALENDAR` are the marker media types. Listing scans resources and filters
by (decrypted) mime type client-side — the server only ever sees ciphertext.

## Design notes

- **No external crate**: a focused, hand-rolled subset avoids adding an iCalendar/vCard
  dependency (and its `cargo audit` surface) for what is a small, stable grammar.
- **Round-trip fidelity over completeness**: unmodelled properties survive via `extra`, so the
  module can grow its explicit coverage without ever having silently dropped data.
- **Typed over existing plumbing**: contacts/events reuse resources rather than adding
  first-class synced entities — smaller and lower-risk, and they inherit all the entity
  machinery for free. The trade-off is a UID-keyed replace-on-edit instead of in-place update.

## Related files

- `keeplin-core/src/models.rs` — `Note` (the to-do fields `CalendarTodo` maps onto).
- Later Front C stages add native contact/event entities and the daemon import/export endpoints
  that call these functions.
