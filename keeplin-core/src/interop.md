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

## Design notes

- **No external crate**: a focused, hand-rolled subset avoids adding an iCalendar/vCard
  dependency (and its `cargo audit` surface) for what is a small, stable grammar.
- **Round-trip fidelity over completeness**: unmodelled properties survive via `extra`, so the
  module can grow its explicit coverage without ever having silently dropped data.

## Related files

- `keeplin-core/src/models.rs` — `Note` (the to-do fields `CalendarTodo` maps onto).
- Later Front C stages add native contact/event entities and the daemon import/export endpoints
  that call these functions.
