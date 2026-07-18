# `interop.rs` — vCard & iCalendar format compatibility

Self-contained companion for `keeplin-core/src/interop.rs`. It documents **every code
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
use chrono::{DateTime, TimeZone, Utc};

use crate::error::StorageError;
use crate::models::{new_id, now, Note, Resource};
use crate::storage::StorageBackend;
```

**What it does** — Standards-format interop: vCard 4.0 (RFC 6350) and iCalendar
`VEVENT`/`VTODO` (RFC 5545). Keeplin is deliberately **not** a WebDAV/CalDAV
server; this module makes it *compatible* with those formats — parsing them in
and serialising them back out — so contacts and calendar items move losslessly
between Keeplin and other apps. Scope: `Contact` ⇄ vCard, `CalendarEvent` ⇄
`VEVENT`, `CalendarTodo` ⇄ `VTODO` (which additionally maps to/from a Keeplin
to-do `Note`), plus `user_vcard` for a profile card. Fidelity: a pragmatic,
widely-interoperable subset of each RFC is modelled explicitly; everything else
in a parsed card/component is preserved verbatim in `extra` lines so a
round-trip never drops properties it doesn't understand. The parsing/formatting
half is pure (no I/O); the second half of the file layers typed contact/event
**storage over resources**:

> "Native" contacts and events are typed at the API level but persist on top of
> the existing **resource** entity, so they ride the sync, encryption,
> permissions and server-materialisation machinery already built — no new entity
> type, table, protobuf message, or sync `Change`. A contact is a resource with
> mime `text/vcard`; an event, `text/calendar`. The stable identity is the
> format `UID` (not the backing resource id), so an edit is a *replace*: a
> soft-delete of the old backing resource plus a fresh one; the tombstones are
> reclaimed by the periodic `purge_deleted_resources` pass
> (`resource_purge_days`). `save_*` always names the backing file `<uid>.vcf` /
> `<uid>.ics`, so by-UID operations find their resource from **metadata alone**;
> resources written before this convention fall back to parsing payloads.

**Dependencies** — `chrono`; `crate::{error, models, storage}`.

**Used by** — `keeplin-daemon/src/rest.rs` (contact/event/todo endpoints and
DTOs) and `server.rs`; tests.

**Repeated context** — Errors follow the crate contract: caller mistakes (no
`VTODO` in an import) are `StorageError::InvalidInput` → HTTP 400 /
`INVALID_ARGUMENT`.

---

## MIME_VCARD

**Identification** — `pub const MIME_VCARD: &str = "text/vcard";` marker
`// md:MIME_VCARD`.

**What it does** — IANA media type marking a resource that backs a `Contact`.

**Dependencies** — none. **Used by** — the contact storage functions;
`rest.rs`. **Repeated context** — none.

---

## MIME_ICALENDAR

**Identification** — `pub const MIME_ICALENDAR: &str = "text/calendar";` marker
`// md:MIME_ICALENDAR`.

**What it does** — IANA media type marking a resource that backs a
`CalendarEvent`.

**Dependencies** — none. **Used by** — the event storage functions; `rest.rs`.
**Repeated context** — none.

---

## fn unfold

**Identification** — `fn unfold(input: &str) -> Vec<String>`; marker
`// md:fn unfold`.

**What it does** — Unfolds RFC 5545/6350 continuation lines: a CRLF (or LF)
followed by a single space or tab is a line fold — removed, rejoining the
wrapped value. Strips `\r` suffixes and drops a trailing empty line from a
final newline.

**Dependencies** — none. **Used by** — `Contact::from_vcard`,
`split_components`. **Repeated context** — none.

---

## fn fold_line

**Identification** — `fn fold_line(line: &str, out: &mut String)`; marker
`// md:fn fold_line`.

**What it does** — Folds a content line at 75 octets with CRLF + leading space
on continuations, per the RFCs; the continuation budget is 74 because the
space counts. Folds only on char boundaries (never mid-UTF-8) so output stays
valid; most consumers are lenient anyway.

**Dependencies** — none. **Used by** — every serialiser in the file.
**Repeated context** — none.

---

## fn escape_text

**Identification** — `fn escape_text(s: &str) -> String`; marker
`// md:fn escape_text`.

**What it does** — Escapes a TEXT value per RFC 5545 §3.3.11 / RFC 6350 §3.4:
`\` `,` `;` backslash-escaped, `\n` → `\\n`, bare `\r` dropped.

**Dependencies** — none. **Used by** — all serialisers.
**Repeated context** — none.

---

## fn unescape_text

**Identification** — `fn unescape_text(s: &str) -> String`; marker
`// md:fn unescape_text`.

**What it does** — Reverses `escape_text`: `\n`/`\N` → newline, any other
escaped char → itself, a trailing lone `\` kept literally.

**Dependencies** — none. **Used by** — all parsers.
**Repeated context** — none.

---

## fn split_prop

**Identification** — `fn split_prop(line: &str) -> Option<(String, &str)>`;
marker `// md:fn split_prop`.

**What it does** — Splits a content line into its property **name**
(upper-cased, parameters after `;` dropped) and raw value:
`SUMMARY;LANGUAGE=en:Hi` → `("SUMMARY", "Hi")`. `None` for a line with no
colon.

**Dependencies** — none. **Used by** — `Contact::from_vcard`,
`split_components`, `parse_component_lines`. **Repeated context** — property
parameters are intentionally not modelled; unknown-property lines are
preserved whole in `extra`, so their parameters survive round-trips anyway.

---

## fn format_dt

**Identification** — `fn format_dt(dt: DateTime<Utc>) -> String`; marker
`// md:fn format_dt`.

**What it does** — RFC 5545 UTC date-time: `YYYYMMDDTHHMMSSZ`.

**Dependencies** — `chrono`. **Used by** — the ICS serialisers,
`write_uid_dtstamp`. **Repeated context** — none.

---

## fn parse_dt

**Identification** — `fn parse_dt(value: &str) -> Option<DateTime<Utc>>`;
marker `// md:fn parse_dt`.

**What it does** — Parses an RFC 5545 date-time or date: `YYYYMMDDTHHMMSSZ`
(the `Z` is a literal — the naive wall-clock is parsed and stamped UTC),
`YYYYMMDDTHHMMSS` (also read as UTC), or a bare `YYYYMMDD` date (midnight
UTC). `None` for anything else.

**Dependencies** — `chrono`. **Used by** — the ICS parsers; tests.
**Repeated context** — time-zone-parameterised values (`;TZID=`) are not
modelled — such lines land in `extra`.

---

## PRODID

**Identification** — `const PRODID: &str = "-//Keeplin//Keeplin//EN";` marker
`// md:PRODID`.

**What it does** — The Keeplin product id stamped on emitted calendars.

**Dependencies** — none. **Used by** — `write_calendar_open`.
**Repeated context** — none.

---

## Contact

**Identification** — struct deriving `Debug, Clone, Default, PartialEq, Eq`;
marker `// md:Contact`.

**What it does** — A contact modelling the widely-used vCard 4.0 subset:
`uid` (`UID`; generated on export when empty), `formatted_name` (`FN`,
spec-required), `family_name`/`given_name` (`N` structured name), `emails`
(`EMAIL`, in order), `phones` (`TEL`), `org` (`ORG`), `note` (`NOTE`),
`extra` (unmodelled property lines kept verbatim for lossless round-trips).

**Dependencies** — none (plain data). **Used by** — the vCard functions, the
contact storage functions, `rest.rs`'s `ContactDto`.
**Repeated context** — none.

---

## impl Contact

**Identification** — inherent impl; marker `// md:impl Contact`. Two methods.

### fn to_vcard

**Identification** — `pub fn to_vcard(&self) -> String`; marker
`// md:impl Contact > fn to_vcard`.

**What it does** — Serialises to a vCard 4.0 card: `BEGIN`/`VERSION:4.0`,
`UID` (fresh `new_id()` when empty), `FN`, `N` (`family;given;;;` — emitted
only when either part is set), each `EMAIL`/`TEL`, `ORG`, `NOTE`, the `extra`
lines verbatim, `END`. Everything escaped and folded.

**Dependencies** — `fold_line`, `escape_text`, `models::new_id`.

**Used by** — `user_vcard`, `save_contact`, tests.

**Repeated context** — none.

### fn from_vcard

**Identification** — `pub fn from_vcard(input: &str) -> Option<Contact>`;
marker `// md:impl Contact > fn from_vcard`.

**What it does** — Parses the **first** `VCARD` in `input`: unfold, then
per-property dispatch (`UID`/`FN`/`N`/`EMAIL`/`TEL`/`ORG`/`NOTE`; `VERSION`
ignored; anything else → `extra`). `N` splits `family;given`, empty parts
become `None`. Returns `None` when no `BEGIN:VCARD` was seen; a card with no
`FN` still round-trips (empty default).

**Dependencies** — `unfold`, `split_prop`, `unescape_text`.

**Used by** — `list_contacts`, `get_contact`, `contact_resources` fallback,
tests.

**Repeated context** — tolerance rule: unknown properties are never a hard
failure.

---

## fn user_vcard

**Identification** — `pub fn user_vcard(display_name: &str, email: &str) -> String`;
marker `// md:fn user_vcard`.

**What it does** — Renders a profile vCard for an account owner (name + one
email) via a default `Contact`.

**Dependencies** — `Contact::to_vcard`.

**Used by** — the daemon's profile endpoint.

**Repeated context** — none.

---

## CalendarEvent

**Identification** — struct deriving `Debug, Clone, Default, PartialEq, Eq`;
marker `// md:CalendarEvent`.

**What it does** — A calendar event modelling the common `VEVENT` subset:
`uid`, `summary`, optional `start` (`DTSTART`) / `end` (`DTEND`) / `location`
/ `description`, and `extra`.

**Dependencies** — `chrono`. **Used by** — the event functions, `rest.rs`'s
`EventDto`. **Repeated context** — none.

---

## CalendarTodo

**Identification** — struct deriving `Debug, Clone, Default, PartialEq, Eq`;
marker `// md:CalendarTodo`.

**What it does** — A calendar to-do modelling the common `VTODO` subset:
`uid`, `summary`, optional `due` (`DUE`) / `completed` (`COMPLETED`) /
`description`, and `extra`. Also the bridge to Keeplin's native to-do notes
(`from_note`/`apply_to_note`).

**Dependencies** — `chrono`. **Used by** — the todo functions,
`import_todo(s)`, `rest.rs`. **Repeated context** — none.

---

## impl CalendarEvent

**Identification** — inherent impl; marker `// md:impl CalendarEvent`. Three
methods.

### fn to_ics

**Identification** — `pub fn to_ics(&self) -> String`; marker
`// md:impl CalendarEvent > fn to_ics`.

**What it does** — Serialises as a full `VCALENDAR` wrapping one `VEVENT`:
calendar open (VERSION/PRODID), `UID`+`DTSTAMP` (both required), `SUMMARY`,
optional `DTSTART`/`DTEND`/`LOCATION`/`DESCRIPTION`, `extra`, closers.

**Dependencies** — `write_calendar_open`, `write_uid_dtstamp`, `fold_line`,
`escape_text`, `format_dt`.

**Used by** — `save_event`, tests.

**Repeated context** — none.

### fn from_ics

**Identification** — `pub fn from_ics(input: &str) -> Option<CalendarEvent>`;
marker `// md:impl CalendarEvent > fn from_ics`.

**What it does** — The **first** `VEVENT` in `input` (delegates to
`from_ics_all`).

**Dependencies** — `from_ics_all`.

**Used by** — `list_events`, `get_event`, `event_resources` fallback.

**Repeated context** — none.

### fn from_ics_all

**Identification** — `pub fn from_ics_all(input: &str) -> Vec<CalendarEvent>`;
marker `// md:impl CalendarEvent > fn from_ics_all`.

**What it does** — **Every** `VEVENT` in document order (empty when none):
`split_components` then per-property dispatch
(`UID`/`SUMMARY`/`DTSTART`/`DTEND`/`LOCATION`/`DESCRIPTION`; `DTSTAMP`
dropped; rest → `extra`).

**Dependencies** — `split_components`, `parse_component_lines`, `parse_dt`,
`unescape_text`.

**Used by** — `from_ics`; whole-calendar import surfaces.

**Repeated context** — none.

---

## impl CalendarTodo

**Identification** — inherent impl; marker `// md:impl CalendarTodo`. Five
methods.

### fn to_ics

**Identification** — marker `// md:impl CalendarTodo > fn to_ics`.

**What it does** — Full `VCALENDAR` wrapping one `VTODO`: `UID`+`DTSTAMP`,
`SUMMARY`, optional `DUE`; a `completed` value additionally emits
`COMPLETED` + `STATUS:COMPLETED` + `PERCENT-COMPLETE:100` (what other clients
expect); optional `DESCRIPTION`; `extra`.

**Dependencies** — `write_calendar_open`, `write_uid_dtstamp`, `fold_line`,
`escape_text`, `format_dt`.

**Used by** — the daemon's todo export; tests.

**Repeated context** — none.

### fn from_ics

**Identification** — marker `// md:impl CalendarTodo > fn from_ics`.

**What it does** — The first `VTODO` (delegates to `from_ics_all`).

**Dependencies** — `from_ics_all`.

**Used by** — `import_todo`.

**Repeated context** — none.

### fn from_ics_all

**Identification** — marker `// md:impl CalendarTodo > fn from_ics_all`.

**What it does** — Every `VTODO` in document order: dispatch over
`UID`/`SUMMARY`/`DUE`/`COMPLETED`/`DESCRIPTION`;
`STATUS`/`PERCENT-COMPLETE`/`DTSTAMP` dropped (derived on export); rest →
`extra`.

**Dependencies** — `split_components`, `parse_component_lines`, `parse_dt`.

**Used by** — `import_todos`, `from_ics`.

**Repeated context** — none.

### fn from_note

**Identification** — `pub fn from_note(note: &Note) -> CalendarTodo`; marker
`// md:impl CalendarTodo > fn from_note`.

**What it does** — A `VTODO` view of a Keeplin to-do note: `title`→`SUMMARY`,
`body`→`DESCRIPTION` (omitted when empty), `todo_due`→`DUE`,
`todo_completed`→`COMPLETED`, note id → `UID`.

**Dependencies** — `Note`.

**Used by** — the daemon's todo export.

**Repeated context** — none.

### fn apply_to_note

**Identification** — `pub fn apply_to_note(&self, note: &mut Note)`; marker
`// md:impl CalendarTodo > fn apply_to_note`.

**What it does** — Applies this `VTODO` onto `note`, marking it a to-do; sets
title, body, `is_todo`, `todo_due`, `todo_completed`. The note's **id is not
changed** — the caller owns identity.

**Dependencies** — `Note`.

**Used by** — `import_todo`, `import_todos`, tests.

**Repeated context** — none.

---

## fn write_calendar_open

**Identification** — `fn write_calendar_open(out: &mut String)`; marker
`// md:fn write_calendar_open`.

**What it does** — Emits `BEGIN:VCALENDAR`, `VERSION:2.0`, `PRODID`.

**Dependencies** — `fold_line`, `PRODID`. **Used by** — both `to_ics` methods.
**Repeated context** — none.

---

## fn write_uid_dtstamp

**Identification** — `fn write_uid_dtstamp(out: &mut String, uid: &str)`;
marker `// md:fn write_uid_dtstamp`.

**What it does** — Writes `UID` (generating a fresh id when empty) and a
`DTSTAMP` of `now()` — both required on a calendar component.

**Dependencies** — `models::new_id`, `models::now`, `format_dt`,
`escape_text`. **Used by** — both `to_ics` methods.
**Repeated context** — none.

---

## fn split_components

**Identification** — `fn split_components(input: &str, kind: &str) -> Vec<Vec<String>>`;
marker `// md:fn split_components`.

**What it does** — Splits `input` into the property lines of **every**
`BEGIN:<kind>` … `END:<kind>` component, in document order. A component left
open by truncated input is still yielded with the lines seen so far (leniency
preserved from the old single-component parser). Real `.ics` exports routinely
bundle many components in one `VCALENDAR`.

**Dependencies** — `unfold`, `split_prop`. **Used by** — both `from_ics_all`
methods. **Repeated context** — none.

---

## fn parse_component_lines

**Identification** —
`fn parse_component_lines(lines: &[String], f: impl FnMut(&str, &str, &str))`;
marker `// md:fn parse_component_lines`.

**What it does** — Drives `f(name, value, raw_line)` over one component's
property lines (skipping colon-less lines).

**Dependencies** — `split_prop`. **Used by** — both `from_ics_all` methods.
**Repeated context** — none.

---

## fn resources_with_mime

**Identification** —
`async fn resources_with_mime(backend, mime) -> Result<Vec<(Resource, Vec<u8>)>, StorageError>`;
marker `// md:fn resources_with_mime`.

**What it does** — Every live resource with `mime`, paired with its bytes:
metadata listing first, then a `read_resource` per hit — read failures are
skipped, not fatal, so a torn concurrent delete never breaks listing.

**Dependencies** — `resource_metas_with_mime`, `read_resource`.

**Used by** — `list_contacts`, `list_events`.

**Repeated context** — listing all contacts/events inherently reads every
payload of that mime; by-UID paths avoid this via the canonical file name.

---

## fn resource_metas_with_mime

**Identification** —
`async fn resource_metas_with_mime(backend, mime) -> Result<Vec<Resource>, StorageError>`;
marker `// md:fn resource_metas_with_mime`.

**What it does** — Every live resource with `mime`, metadata only: exhausts
the cursor-paginated `list_resources` and filters by `mime_type`.

**Dependencies** — `list_resources`.

**Used by** — `resources_with_mime`, `resources_with_uid`.

**Repeated context** — under `EncryptedBackend` the metadata page is already
decrypted, so the `mime_type` comparison works on plaintext.

---

## fn uid_file_name

**Identification** — `fn uid_file_name(uid: &str, ext: &str) -> String`;
marker `// md:fn uid_file_name`.

**What it does** — The canonical backing file name `save_*` writes for a UID
(`{uid}.{ext}`) — the metadata-level key every by-UID lookup tries first.

**Dependencies** — none. **Used by** — `resources_with_uid`, `save_contact`,
`save_event`. **Repeated context** — the file name is load-bearing: changing
the convention breaks the metadata fast path (the payload-parsing fallback
still works).

---

## fn resources_with_uid

**Identification** —
`async fn resources_with_uid(backend, mime, ext, uid, uid_of) -> Result<Vec<Resource>, StorageError>`;
marker `// md:fn resources_with_uid`.

**What it does** — The backing resources of `uid`: filter metadata by the
canonical file name (fast path, no blob reads); when nothing matches, the
legacy fallback parses every payload of that mime with `uid_of` (a closure
extracting the UID from bytes).

**Dependencies** — `resource_metas_with_mime`, `uid_file_name`,
`read_resource`.

**Used by** — `contact_resources`, `event_resources`.

**Repeated context** — none.

---

## fn save_contact

**Identification** —
`pub async fn save_contact(backend, mut contact: Contact) -> Result<Contact, StorageError>`;
marker `// md:fn save_contact`.

**What it does** — Upsert by vCard `UID` (generating one when empty): delete
any existing contact resource with the same UID, then write a fresh
`text/vcard` resource titled with the formatted name and named `<uid>.vcf`
(the load-bearing canonical name). Returns the stored contact with its UID
populated.

**Dependencies** — `delete_contact`, `Contact::to_vcard`, `uid_file_name`,
`Resource::new`, `create_resource`.

**Used by** — the daemon's contact endpoints; tests.

**Repeated context** — edit-as-replace leaves tombstones behind by design;
`purge_deleted_resources` reclaims the dead bytes later.

---

## fn list_contacts

**Identification** —
`pub async fn list_contacts(backend) -> Result<Vec<Contact>, StorageError>`;
marker `// md:fn list_contacts`.

**What it does** — Every stored contact, parsed from its backing vCard
resource (unparseable payloads are skipped).

**Dependencies** — `resources_with_mime`, `Contact::from_vcard`.

**Used by** — the daemon's contact listing.

**Repeated context** — none.

---

## fn contact_resources

**Identification** —
`async fn contact_resources(backend, uid) -> Result<Vec<Resource>, StorageError>`;
marker `// md:fn contact_resources`.

**What it does** — The backing resources of the contact `uid` (metadata only;
usually one) — `resources_with_uid` specialised with the vCard parser as the
fallback UID extractor.

**Dependencies** — `resources_with_uid`, `Contact::from_vcard`.

**Used by** — `get_contact`, `delete_contact`.

**Repeated context** — none.

---

## fn get_contact

**Identification** —
`pub async fn get_contact(backend, uid) -> Result<Option<Contact>, StorageError>`;
marker `// md:fn get_contact`.

**What it does** — The stored contact with `uid`, if any: reads only that
contact's backing resource(s), re-verifying the parsed UID before returning.

**Dependencies** — `contact_resources`, `read_resource`,
`Contact::from_vcard`.

**Used by** — the daemon's contact read.

**Repeated context** — none.

---

## fn delete_contact

**Identification** —
`pub async fn delete_contact(backend, uid) -> Result<(), StorageError>`;
marker `// md:fn delete_contact`.

**What it does** — Soft-deletes every contact resource carrying `uid` (usually
one); a no-op when none match.

**Dependencies** — `contact_resources`, `delete_resource`.

**Used by** — `save_contact` (the replace step); the daemon's delete endpoint.

**Repeated context** — none.

---

## fn save_event

**Identification** —
`pub async fn save_event(backend, mut event) -> Result<CalendarEvent, StorageError>`;
marker `// md:fn save_event`.

**What it does** — Event twin of `save_contact`: upsert by iCalendar `UID`,
backing resource `text/calendar` named `<uid>.ics`, titled with the summary.

**Dependencies** — `delete_event`, `CalendarEvent::to_ics`, `uid_file_name`,
`create_resource`.

**Used by** — the daemon's event endpoints; tests.

**Repeated context** — none.

---

## fn list_events

**Identification** —
`pub async fn list_events(backend) -> Result<Vec<CalendarEvent>, StorageError>`;
marker `// md:fn list_events`.

**What it does** — Every stored event, parsed from its backing iCalendar
resource.

**Dependencies** — `resources_with_mime`, `CalendarEvent::from_ics`.

**Used by** — the daemon's event listing.

**Repeated context** — none.

---

## fn event_resources

**Identification** —
`async fn event_resources(backend, uid) -> Result<Vec<Resource>, StorageError>`;
marker `// md:fn event_resources`.

**What it does** — The backing resources of the event `uid` —
`resources_with_uid` specialised with the ICS parser as the fallback.

**Dependencies** — `resources_with_uid`, `CalendarEvent::from_ics`.

**Used by** — `get_event`, `delete_event`.

**Repeated context** — none.

---

## fn get_event

**Identification** —
`pub async fn get_event(backend, uid) -> Result<Option<CalendarEvent>, StorageError>`;
marker `// md:fn get_event`.

**What it does** — The stored event with `uid`, if any; reads only its backing
resource(s), re-verifying the parsed UID.

**Dependencies** — `event_resources`, `read_resource`,
`CalendarEvent::from_ics`.

**Used by** — the daemon's event read.

**Repeated context** — none.

---

## fn delete_event

**Identification** —
`pub async fn delete_event(backend, uid) -> Result<(), StorageError>`;
marker `// md:fn delete_event`.

**What it does** — Soft-deletes every event resource carrying `uid`; a no-op
when none match.

**Dependencies** — `event_resources`, `delete_resource`.

**Used by** — `save_event`; the daemon's delete endpoint.

**Repeated context** — none.

---

## fn import_todo

**Identification** —
`pub async fn import_todo(backend, ics) -> Result<Note, StorageError>`;
marker `// md:fn import_todo`.

**What it does** — Imports the **first** `VTODO` in `ics` as a Keeplin to-do
**note** (the native mapping — unlike contacts/events, a to-do is a
first-class note, not a resource). `InvalidInput("no VTODO in input")` when
none. Returns the created note.

**Dependencies** — `CalendarTodo::from_ics`, `apply_to_note`, `create_note`.

**Used by** — the daemon's import endpoint; tests.

**Repeated context** — the created note starts in the Inbox; the daemon's
create path runs placement (`ordering::place_new_note`) around this.

---

## fn import_todos

**Identification** —
`pub async fn import_todos(backend, ics) -> Result<Vec<Note>, StorageError>`;
marker `// md:fn import_todos`.

**What it does** — Imports **every** `VTODO` in the input as to-do notes, in
document order; errors with `InvalidInput` when the input carries none at all.
Sequential creates — a mid-batch failure leaves earlier notes created.

**Dependencies** — `CalendarTodo::from_ics_all`, `apply_to_note`,
`create_note`.

**Used by** — the daemon's bulk import.

**Repeated context** — none.

---

## mod tests

**Identification** — `#[cfg(test)]` test module; marker `// md:mod tests`. One
helper + twelve tests (nine pure, three over a real `FsBackend`).

**What it does** — Round-trip fidelity, escaping, folding, multi-component
parsing, the note bridge, and the storage layer end-to-end.

**Dependencies** — `super::*`, `models::Note`, `tempfile`, `tokio`.

**Used by** — CI.

**Repeated context** — none.

### fn contact_round_trips_through_vcard

**Identification** — unit test; marker
`// md:mod tests > fn contact_round_trips_through_vcard`.

**What it does** — A fully-populated `Contact` (multiple emails, an org with
`;`, a multi-line note with a comma, an `X-CUSTOM` extra) survives
`to_vcard → from_vcard` exactly equal.

### fn vcard_escaping_survives

**Identification** — unit test; marker
`// md:mod tests > fn vcard_escaping_survives`.

**What it does** — `A, B; C\D` in `FN` round-trips exactly.

### fn user_vcard_carries_name_and_email

**Identification** — unit test; marker
`// md:mod tests > fn user_vcard_carries_name_and_email`.

**What it does** — The profile card parses back with the given name and email.

### fn event_round_trips_through_ics

**Identification** — unit test; marker
`// md:mod tests > fn event_round_trips_through_ics`.

**What it does** — A full `CalendarEvent` survives `to_ics → from_ics`.

### fn todo_round_trips_and_marks_completion

**Identification** — unit test; marker
`// md:mod tests > fn todo_round_trips_and_marks_completion`.

**What it does** — A completed `CalendarTodo` emits `STATUS:COMPLETED` and
round-trips equal.

### fn todo_maps_to_and_from_a_note

**Identification** — unit test; marker
`// md:mod tests > fn todo_maps_to_and_from_a_note`.

**What it does** — `from_note` copies title/uid/due; `apply_to_note` onto a
blank note sets `is_todo`, title, body, due.

### fn unfolds_wrapped_lines

**Identification** — unit test; marker
`// md:mod tests > fn unfolds_wrapped_lines`.

**What it does** — A 200-char `DESCRIPTION` (long enough to fold) rejoins on
parse.

### fn missing_component_yields_none

**Identification** — unit test; marker
`// md:mod tests > fn missing_component_yields_none`.

**What it does** — Non-calendar/non-card input → `None`/empty, never a panic.

### fn multi_component_calendar_parses_every_event_and_todo

**Identification** — unit test; marker
`// md:mod tests > fn multi_component_calendar_parses_every_event_and_todo`.

**What it does** — Splices two `VEVENT`s and one `VTODO` into a single
`VCALENDAR`: `from_ics_all` yields both events in order plus the todo;
`from_ics` keeps its first-component behaviour.

### fn fs

**Identification** — helper `async fn fs() -> FsBackend`; marker
`// md:mod tests > fn fs`.

**What it does** — An `FsBackend` in a kept tempdir.

### fn contact_save_list_get_delete_over_storage

**Identification** — tokio test; marker
`// md:mod tests > fn contact_save_list_get_delete_over_storage`.

**What it does** — Save assigns a UID; list shows one; a re-save with the same
UID **replaces** (still one, updated name); get finds it; delete empties the
list.

### fn event_round_trips_through_storage

**Identification** — tokio test; marker
`// md:mod tests > fn event_round_trips_through_storage`.

**What it does** — `save_event` then `get_event` returns the summary and
start.

### fn import_todo_creates_a_todo_note

**Identification** — tokio test; marker
`// md:mod tests > fn import_todo_creates_a_todo_note`.

**What it does** — Importing a `VTODO` ICS creates a note with `is_todo`,
title, and due set.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `resources_with_mime()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `resource_metas_with_mime()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `resources_with_uid()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `save_contact()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `contact_resources()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `save_event()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `event_resources()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `import_todo()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `import_todos()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `Contact` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — error types (EXTRACTED: imports_from×1, references×15; e.g. `StorageError`)
- `keeplin-core/src/models.rs` — domain data types (EXTRACTED: calls×2, imports_from×1, references×9; e.g. `Note`, `Resource`, `new_id()`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: imports_from×1, references×15; e.g. `StorageBackend`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references×1; e.g. `FsBackend`)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-daemon/src/rest.rs` — REST/JSON API + WebSocket feed (axum) (EXTRACTED: references×4; e.g. `ContactDto`, `.from()`, `EventDto`)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `const MIME_VCARD` | `// md:MIME_VCARD` |
| 3 | `const MIME_ICALENDAR` | `// md:MIME_ICALENDAR` |
| 4 | `fn unfold` | `// md:fn unfold` |
| 5 | `fn fold_line` | `// md:fn fold_line` |
| 6 | `fn escape_text` | `// md:fn escape_text` |
| 7 | `fn unescape_text` | `// md:fn unescape_text` |
| 8 | `fn split_prop` | `// md:fn split_prop` |
| 9 | `fn format_dt` | `// md:fn format_dt` |
| 10 | `fn parse_dt` | `// md:fn parse_dt` |
| 11 | `const PRODID` | `// md:PRODID` |
| 12 | `struct Contact` | `// md:Contact` |
| 13 | `impl Contact` (+ `to_vcard`, `from_vcard`) | `// md:impl Contact` (+ `> fn …`) |
| 14 | `fn user_vcard` | `// md:fn user_vcard` |
| 15 | `struct CalendarEvent` | `// md:CalendarEvent` |
| 16 | `struct CalendarTodo` | `// md:CalendarTodo` |
| 17 | `impl CalendarEvent` (+ `to_ics`, `from_ics`, `from_ics_all`) | `// md:impl CalendarEvent` (+ `> fn …`) |
| 18 | `impl CalendarTodo` (+ `to_ics`, `from_ics`, `from_ics_all`, `from_note`, `apply_to_note`) | `// md:impl CalendarTodo` (+ `> fn …`) |
| 19 | `fn write_calendar_open` | `// md:fn write_calendar_open` |
| 20 | `fn write_uid_dtstamp` | `// md:fn write_uid_dtstamp` |
| 21 | `fn split_components` | `// md:fn split_components` |
| 22 | `fn parse_component_lines` | `// md:fn parse_component_lines` |
| 23 | `fn resources_with_mime` | `// md:fn resources_with_mime` |
| 24 | `fn resource_metas_with_mime` | `// md:fn resource_metas_with_mime` |
| 25 | `fn uid_file_name` | `// md:fn uid_file_name` |
| 26 | `fn resources_with_uid` | `// md:fn resources_with_uid` |
| 27 | `fn save_contact` | `// md:fn save_contact` |
| 28 | `fn list_contacts` | `// md:fn list_contacts` |
| 29 | `fn contact_resources` | `// md:fn contact_resources` |
| 30 | `fn get_contact` | `// md:fn get_contact` |
| 31 | `fn delete_contact` | `// md:fn delete_contact` |
| 32 | `fn save_event` | `// md:fn save_event` |
| 33 | `fn list_events` | `// md:fn list_events` |
| 34 | `fn event_resources` | `// md:fn event_resources` |
| 35 | `fn get_event` | `// md:fn get_event` |
| 36 | `fn delete_event` | `// md:fn delete_event` |
| 37 | `fn import_todo` | `// md:fn import_todo` |
| 38 | `fn import_todos` | `// md:fn import_todos` |
| 39 | `mod tests` (+ helper `fs` + twelve tests) | `// md:mod tests` (+ `> fn …`) |
