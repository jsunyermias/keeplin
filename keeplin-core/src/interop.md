# `interop.rs` — vCard & iCalendar format compatibility

Self-contained companion for `keeplin-core/src/interop.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able
to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use chrono::{DateTime, TimeZone, Utc};

use crate::error::StorageError;
use crate::models::{new_id, now, Note, Resource, SYSTEM_RESOURCE_NOTE_ID};
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

**Code** — complete and verbatim:

```rust
// md:MIME_VCARD
pub const MIME_VCARD: &str = "text/vcard";
```

**What it does** — IANA media type marking a resource that backs a `Contact`.

**Dependencies** — none. **Used by** — the contact storage functions;
`rest.rs`. **Repeated context** — none.

---

## MIME_ICALENDAR

**Identification** — `pub const MIME_ICALENDAR: &str = "text/calendar";` marker
`// md:MIME_ICALENDAR`.

**Code** — complete and verbatim:

```rust
// md:MIME_ICALENDAR
pub const MIME_ICALENDAR: &str = "text/calendar";
```

**What it does** — IANA media type marking a resource that backs a
`CalendarEvent`.

**Dependencies** — none. **Used by** — the event storage functions; `rest.rs`.
**Repeated context** — none.

---

## fn unfold

**Identification** — `fn unfold(input: &str) -> Vec<String>`; marker
`// md:fn unfold`.

**Code** — complete and verbatim:

```rust
// md:fn unfold
fn unfold(input: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in input.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_string());
    }
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn fold_line
fn fold_line(line: &str, out: &mut String) {
    const LIMIT: usize = 75;
    if line.len() <= LIMIT {
        out.push_str(line);
        out.push_str("\r\n");
        return;
    }
    let mut start = 0;
    let mut first = true;
    while start < line.len() {
        let budget = if first { LIMIT } else { LIMIT - 1 };
        let mut end = (start + budget).min(line.len());
        while end < line.len() && !line.is_char_boundary(end) {
            end -= 1;
        }
        if !first {
            out.push(' ');
        }
        out.push_str(&line[start..end]);
        out.push_str("\r\n");
        start = end;
        first = false;
    }
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn escape_text
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}
```

**What it does** — Escapes a TEXT value per RFC 5545 §3.3.11 / RFC 6350 §3.4:
`\` `,` `;` backslash-escaped, `\n` → `\\n`, bare `\r` dropped.

**Dependencies** — none. **Used by** — all serialisers.
**Repeated context** — none.

---

## fn unescape_text

**Identification** — `fn unescape_text(s: &str) -> String`; marker
`// md:fn unescape_text`.

**Code** — complete and verbatim:

```rust
// md:fn unescape_text
fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
```

**What it does** — Reverses `escape_text`: `\n`/`\N` → newline, any other
escaped char → itself, a trailing lone `\` kept literally.

**Dependencies** — none. **Used by** — all parsers.
**Repeated context** — none.

---

## fn split_prop

**Identification** — `fn split_prop(line: &str) -> Option<(String, &str)>`;
marker `// md:fn split_prop`.

**Code** — complete and verbatim:

```rust
// md:fn split_prop
fn split_prop(line: &str) -> Option<(String, &str)> {
    let colon = line.find(':')?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let name_end = head.find(';').unwrap_or(head.len());
    Some((head[..name_end].to_ascii_uppercase(), value))
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn format_dt
fn format_dt(dt: DateTime<Utc>) -> String {
    dt.format("%Y%m%dT%H%M%SZ").to_string()
}
```

**What it does** — RFC 5545 UTC date-time: `YYYYMMDDTHHMMSSZ`.

**Dependencies** — `chrono`. **Used by** — the ICS serialisers,
`write_uid_dtstamp`. **Repeated context** — none.

---

## fn parse_dt

**Identification** — `fn parse_dt(value: &str) -> Option<DateTime<Utc>>`;
marker `// md:fn parse_dt`.

**Code** — complete and verbatim:

```rust
// md:fn parse_dt
fn parse_dt(value: &str) -> Option<DateTime<Utc>> {
    let v = value.trim();
    for fmt in ["%Y%m%dT%H%M%SZ", "%Y%m%dT%H%M%S"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(v, fmt) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(v, "%Y%m%d") {
        return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?));
    }
    None
}
```

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

**Code** — complete and verbatim:

```rust
// md:PRODID
const PRODID: &str = "-//Keeplin//Keeplin//EN";
```

**What it does** — The Keeplin product id stamped on emitted calendars.

**Dependencies** — none. **Used by** — `write_calendar_open`.
**Repeated context** — none.

---

## Contact

**Identification** — struct deriving `Debug, Clone, Default, PartialEq, Eq`;
marker `// md:Contact`.

**Code** — complete and verbatim:

```rust
// md:Contact
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contact {
    pub uid: String,
    pub formatted_name: String,
    pub family_name: Option<String>,
    pub given_name: Option<String>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub org: Option<String>,
    pub note: Option<String>,
    pub extra: Vec<String>,
}
```

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

**Code** — container: members documented as sub-blocks below: fn to_vcard, fn from_vcard.

### fn to_vcard

**Identification** — `pub fn to_vcard(&self) -> String`; marker
`// md:impl Contact > fn to_vcard`.

**Code** — complete and verbatim:

```rust
    // md:impl Contact > fn to_vcard
    pub fn to_vcard(&self) -> String {
        let mut out = String::new();
        fold_line("BEGIN:VCARD", &mut out);
        fold_line("VERSION:4.0", &mut out);
        let uid = if self.uid.is_empty() {
            crate::models::new_id().to_string()
        } else {
            self.uid.clone()
        };
        fold_line(&format!("UID:{}", escape_text(&uid)), &mut out);
        fold_line(
            &format!("FN:{}", escape_text(&self.formatted_name)),
            &mut out,
        );
        if self.family_name.is_some() || self.given_name.is_some() {
            let fam = self.family_name.as_deref().unwrap_or("");
            let giv = self.given_name.as_deref().unwrap_or("");
            fold_line(
                &format!("N:{};{};;;", escape_text(fam), escape_text(giv)),
                &mut out,
            );
        }
        for email in &self.emails {
            fold_line(&format!("EMAIL:{}", escape_text(email)), &mut out);
        }
        for tel in &self.phones {
            fold_line(&format!("TEL:{}", escape_text(tel)), &mut out);
        }
        if let Some(org) = &self.org {
            fold_line(&format!("ORG:{}", escape_text(org)), &mut out);
        }
        if let Some(note) = &self.note {
            fold_line(&format!("NOTE:{}", escape_text(note)), &mut out);
        }
        for line in &self.extra {
            fold_line(line, &mut out);
        }
        fold_line("END:VCARD", &mut out);
        out
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl Contact > fn from_vcard
    pub fn from_vcard(input: &str) -> Option<Contact> {
        let lines = unfold(input);
        let mut in_card = false;
        let mut c = Contact::default();
        for line in &lines {
            let (name, value) = match split_prop(line) {
                Some(p) => p,
                None => continue,
            };
            match name.as_str() {
                "BEGIN" if value.eq_ignore_ascii_case("VCARD") => in_card = true,
                "END" if value.eq_ignore_ascii_case("VCARD") => break,
                _ if !in_card => {}
                "VERSION" => {}
                "UID" => c.uid = unescape_text(value),
                "FN" => c.formatted_name = unescape_text(value),
                "N" => {
                    let parts: Vec<&str> = value.splitn(2, ';').collect();
                    c.family_name = parts
                        .first()
                        .map(|s| unescape_text(s))
                        .filter(|s| !s.is_empty());
                    c.given_name = parts
                        .get(1)
                        .and_then(|s| s.split(';').next())
                        .map(unescape_text)
                        .filter(|s| !s.is_empty());
                }
                "EMAIL" => c.emails.push(unescape_text(value)),
                "TEL" => c.phones.push(unescape_text(value)),
                "ORG" => c.org = Some(unescape_text(value)),
                "NOTE" => c.note = Some(unescape_text(value)),
                _ => c.extra.push(line.clone()),
            }
        }
        if in_card {
            Some(c)
        } else {
            None
        }
    }
```

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

**Code** — complete and verbatim:

```rust
// md:fn user_vcard
pub fn user_vcard(display_name: &str, email: &str) -> String {
    Contact {
        formatted_name: display_name.to_string(),
        emails: vec![email.to_string()],
        ..Default::default()
    }
    .to_vcard()
}
```

**What it does** — Renders a profile vCard for an account owner (name + one
email) via a default `Contact`.

**Dependencies** — `Contact::to_vcard`.

**Used by** — the daemon's profile endpoint.

**Repeated context** — none.

---

## CalendarEvent

**Identification** — struct deriving `Debug, Clone, Default, PartialEq, Eq`;
marker `// md:CalendarEvent`.

**Code** — complete and verbatim:

```rust
// md:CalendarEvent
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarEvent {
    pub uid: String,
    pub summary: String,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub extra: Vec<String>,
}
```

**What it does** — A calendar event modelling the common `VEVENT` subset:
`uid`, `summary`, optional `start` (`DTSTART`) / `end` (`DTEND`) / `location`
/ `description`, and `extra`.

**Dependencies** — `chrono`. **Used by** — the event functions, `rest.rs`'s
`EventDto`. **Repeated context** — none.

---

## CalendarTodo

**Identification** — struct deriving `Debug, Clone, Default, PartialEq, Eq`;
marker `// md:CalendarTodo`.

**Code** — complete and verbatim:

```rust
// md:CalendarTodo
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarTodo {
    pub uid: String,
    pub summary: String,
    pub due: Option<DateTime<Utc>>,
    pub completed: Option<DateTime<Utc>>,
    pub description: Option<String>,
    pub extra: Vec<String>,
}
```

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

**Code** — container: members documented as sub-blocks below: fn to_ics, fn from_ics, fn from_ics_all.

### fn to_ics

**Identification** — `pub fn to_ics(&self) -> String`; marker
`// md:impl CalendarEvent > fn to_ics`.

**Code** — complete and verbatim:

```rust
    // md:impl CalendarEvent > fn to_ics
    pub fn to_ics(&self) -> String {
        let mut out = String::new();
        write_calendar_open(&mut out);
        fold_line("BEGIN:VEVENT", &mut out);
        write_uid_dtstamp(&mut out, &self.uid);
        fold_line(&format!("SUMMARY:{}", escape_text(&self.summary)), &mut out);
        if let Some(s) = self.start {
            fold_line(&format!("DTSTART:{}", format_dt(s)), &mut out);
        }
        if let Some(e) = self.end {
            fold_line(&format!("DTEND:{}", format_dt(e)), &mut out);
        }
        if let Some(loc) = &self.location {
            fold_line(&format!("LOCATION:{}", escape_text(loc)), &mut out);
        }
        if let Some(desc) = &self.description {
            fold_line(&format!("DESCRIPTION:{}", escape_text(desc)), &mut out);
        }
        for line in &self.extra {
            fold_line(line, &mut out);
        }
        fold_line("END:VEVENT", &mut out);
        fold_line("END:VCALENDAR", &mut out);
        out
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl CalendarEvent > fn from_ics
    pub fn from_ics(input: &str) -> Option<CalendarEvent> {
        Self::from_ics_all(input).into_iter().next()
    }
```

**What it does** — The **first** `VEVENT` in `input` (delegates to
`from_ics_all`).

**Dependencies** — `from_ics_all`.

**Used by** — `list_events`, `get_event`, `event_resources` fallback.

**Repeated context** — none.

### fn from_ics_all

**Identification** — `pub fn from_ics_all(input: &str) -> Vec<CalendarEvent>`;
marker `// md:impl CalendarEvent > fn from_ics_all`.

**Code** — complete and verbatim:

```rust
    // md:impl CalendarEvent > fn from_ics_all
    pub fn from_ics_all(input: &str) -> Vec<CalendarEvent> {
        split_components(input, "VEVENT")
            .into_iter()
            .map(|lines| {
                let mut ev = CalendarEvent::default();
                parse_component_lines(&lines, |name, value, line| match name {
                    "UID" => ev.uid = unescape_text(value),
                    "SUMMARY" => ev.summary = unescape_text(value),
                    "DTSTART" => ev.start = parse_dt(value),
                    "DTEND" => ev.end = parse_dt(value),
                    "LOCATION" => ev.location = Some(unescape_text(value)),
                    "DESCRIPTION" => ev.description = Some(unescape_text(value)),
                    "DTSTAMP" => {}
                    _ => ev.extra.push(line.to_string()),
                });
                ev
            })
            .collect()
    }
```

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

**Code** — container: members documented as sub-blocks below: fn to_ics, fn from_ics, fn from_ics_all, fn from_note, fn apply_to_note.

### fn to_ics

**Identification** — marker `// md:impl CalendarTodo > fn to_ics`.

**Code** — complete and verbatim:

```rust
    // md:impl CalendarTodo > fn to_ics
    pub fn to_ics(&self) -> String {
        let mut out = String::new();
        write_calendar_open(&mut out);
        fold_line("BEGIN:VTODO", &mut out);
        write_uid_dtstamp(&mut out, &self.uid);
        fold_line(&format!("SUMMARY:{}", escape_text(&self.summary)), &mut out);
        if let Some(due) = self.due {
            fold_line(&format!("DUE:{}", format_dt(due)), &mut out);
        }
        if let Some(done) = self.completed {
            fold_line(&format!("COMPLETED:{}", format_dt(done)), &mut out);
            fold_line("STATUS:COMPLETED", &mut out);
            fold_line("PERCENT-COMPLETE:100", &mut out);
        }
        if let Some(desc) = &self.description {
            fold_line(&format!("DESCRIPTION:{}", escape_text(desc)), &mut out);
        }
        for line in &self.extra {
            fold_line(line, &mut out);
        }
        fold_line("END:VTODO", &mut out);
        fold_line("END:VCALENDAR", &mut out);
        out
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl CalendarTodo > fn from_ics
    pub fn from_ics(input: &str) -> Option<CalendarTodo> {
        Self::from_ics_all(input).into_iter().next()
    }
```

**What it does** — The first `VTODO` (delegates to `from_ics_all`).

**Dependencies** — `from_ics_all`.

**Used by** — `import_todo`.

**Repeated context** — none.

### fn from_ics_all

**Identification** — marker `// md:impl CalendarTodo > fn from_ics_all`.

**Code** — complete and verbatim:

```rust
    // md:impl CalendarTodo > fn from_ics_all
    pub fn from_ics_all(input: &str) -> Vec<CalendarTodo> {
        split_components(input, "VTODO")
            .into_iter()
            .map(|lines| {
                let mut td = CalendarTodo::default();
                parse_component_lines(&lines, |name, value, line| match name {
                    "UID" => td.uid = unescape_text(value),
                    "SUMMARY" => td.summary = unescape_text(value),
                    "DUE" => td.due = parse_dt(value),
                    "COMPLETED" => td.completed = parse_dt(value),
                    "DESCRIPTION" => td.description = Some(unescape_text(value)),
                    "STATUS" | "PERCENT-COMPLETE" | "DTSTAMP" => {}
                    _ => td.extra.push(line.to_string()),
                });
                td
            })
            .collect()
    }
```

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

**Code** — complete and verbatim:

```rust
    // md:impl CalendarTodo > fn from_note
    pub fn from_note(note: &Note) -> CalendarTodo {
        CalendarTodo {
            uid: note.id.to_string(),
            summary: note.title.clone(),
            due: note.todo_due,
            completed: note.todo_completed,
            description: (!note.body.is_empty()).then(|| note.body.clone()),
            extra: Vec::new(),
        }
    }
```

**What it does** — A `VTODO` view of a Keeplin to-do note: `title`→`SUMMARY`,
`body`→`DESCRIPTION` (omitted when empty), `todo_due`→`DUE`,
`todo_completed`→`COMPLETED`, note id → `UID`.

**Dependencies** — `Note`.

**Used by** — the daemon's todo export.

**Repeated context** — none.

### fn apply_to_note

**Identification** — `pub fn apply_to_note(&self, note: &mut Note)`; marker
`// md:impl CalendarTodo > fn apply_to_note`.

**Code** — complete and verbatim:

```rust
    // md:impl CalendarTodo > fn apply_to_note
    pub fn apply_to_note(&self, note: &mut Note) {
        note.title = self.summary.clone();
        note.body = self.description.clone().unwrap_or_default();
        note.is_todo = true;
        note.todo_due = self.due;
        note.todo_completed = self.completed;
    }
```

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

**Code** — complete and verbatim:

```rust
// md:fn write_calendar_open
fn write_calendar_open(out: &mut String) {
    fold_line("BEGIN:VCALENDAR", out);
    fold_line("VERSION:2.0", out);
    fold_line(&format!("PRODID:{PRODID}"), out);
}
```

**What it does** — Emits `BEGIN:VCALENDAR`, `VERSION:2.0`, `PRODID`.

**Dependencies** — `fold_line`, `PRODID`. **Used by** — both `to_ics` methods.
**Repeated context** — none.

---

## fn write_uid_dtstamp

**Identification** — `fn write_uid_dtstamp(out: &mut String, uid: &str)`;
marker `// md:fn write_uid_dtstamp`.

**Code** — complete and verbatim:

```rust
// md:fn write_uid_dtstamp
fn write_uid_dtstamp(out: &mut String, uid: &str) {
    let uid = if uid.is_empty() {
        crate::models::new_id().to_string()
    } else {
        uid.to_string()
    };
    fold_line(&format!("UID:{}", escape_text(&uid)), out);
    fold_line(&format!("DTSTAMP:{}", format_dt(now())), out);
}
```

**What it does** — Writes `UID` (generating a fresh id when empty) and a
`DTSTAMP` of `now()` — both required on a calendar component.

**Dependencies** — `models::new_id`, `models::now`, `format_dt`,
`escape_text`. **Used by** — both `to_ics` methods.
**Repeated context** — none.

---

## fn split_components

**Identification** — `fn split_components(input: &str, kind: &str) -> Vec<Vec<String>>`;
marker `// md:fn split_components`.

**Code** — complete and verbatim:

```rust
// md:fn split_components
fn split_components(input: &str, kind: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in unfold(input) {
        let (name, value) = match split_prop(&line) {
            Some(p) => p,
            None => continue,
        };
        if name == "BEGIN" && value.eq_ignore_ascii_case(kind) {
            current = Some(Vec::new());
            continue;
        }
        if name == "END" && value.eq_ignore_ascii_case(kind) {
            if let Some(lines) = current.take() {
                out.push(lines);
            }
            continue;
        }
        if let Some(lines) = current.as_mut() {
            lines.push(line);
        }
    }
    if let Some(lines) = current.take() {
        out.push(lines);
    }
    out
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn parse_component_lines
fn parse_component_lines(lines: &[String], mut f: impl FnMut(&str, &str, &str)) {
    for line in lines {
        if let Some((name, value)) = split_prop(line) {
            f(&name, value, line);
        }
    }
}
```

**What it does** — Drives `f(name, value, raw_line)` over one component's
property lines (skipping colon-less lines).

**Dependencies** — `split_prop`. **Used by** — both `from_ics_all` methods.
**Repeated context** — none.

---

## fn resources_with_mime

**Identification** —
`async fn resources_with_mime(backend, mime) -> Result<Vec<(Resource, Vec<u8>)>, StorageError>`;
marker `// md:fn resources_with_mime`.

**Code** — complete and verbatim:

```rust
// md:fn resources_with_mime
async fn resources_with_mime(
    backend: &dyn StorageBackend,
    mime: &str,
) -> Result<Vec<(Resource, Vec<u8>)>, StorageError> {
    let mut out = Vec::new();
    for meta in resource_metas_with_mime(backend, mime).await? {
        if let Ok(pair) = backend.read_resource(meta.id).await {
            out.push(pair);
        }
    }
    Ok(out)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn resource_metas_with_mime
async fn resource_metas_with_mime(
    backend: &dyn StorageBackend,
    mime: &str,
) -> Result<Vec<Resource>, StorageError> {
    let mut out = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = backend.list_resources(0, token).await?;
        out.extend(page.into_iter().filter(|meta| meta.mime_type == mime));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    Ok(out)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn uid_file_name
fn uid_file_name(uid: &str, ext: &str) -> String {
    format!("{uid}.{ext}")
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn resources_with_uid
async fn resources_with_uid(
    backend: &dyn StorageBackend,
    mime: &str,
    ext: &str,
    uid: &str,
    uid_of: impl Fn(&[u8]) -> Option<String>,
) -> Result<Vec<Resource>, StorageError> {
    let metas = resource_metas_with_mime(backend, mime).await?;
    let canonical = uid_file_name(uid, ext);
    let named: Vec<Resource> = metas
        .iter()
        .filter(|m| m.file_name == canonical)
        .cloned()
        .collect();
    if !named.is_empty() {
        return Ok(named);
    }
    let mut matched = Vec::new();
    for meta in metas {
        if let Ok((_, data)) = backend.read_resource(meta.id).await {
            if uid_of(&data).as_deref() == Some(uid) {
                matched.push(meta);
            }
        }
    }
    Ok(matched)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn save_contact
pub async fn save_contact(
    backend: &dyn StorageBackend,
    mut contact: Contact,
) -> Result<Contact, StorageError> {
    if contact.uid.is_empty() {
        contact.uid = new_id().to_string();
    }
    delete_contact(backend, &contact.uid).await?;
    let vcard = contact.to_vcard();
    let res = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        contact.formatted_name.clone(),
        MIME_VCARD,
        uid_file_name(&contact.uid, "vcf"),
        vcard.len() as u64,
    );
    backend.create_resource(res, vcard.into_bytes()).await?;
    Ok(contact)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn list_contacts
pub async fn list_contacts(backend: &dyn StorageBackend) -> Result<Vec<Contact>, StorageError> {
    let mut contacts = Vec::new();
    for (_, data) in resources_with_mime(backend, MIME_VCARD).await? {
        if let Some(c) = Contact::from_vcard(&String::from_utf8_lossy(&data)) {
            contacts.push(c);
        }
    }
    Ok(contacts)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn contact_resources
async fn contact_resources(
    backend: &dyn StorageBackend,
    uid: &str,
) -> Result<Vec<Resource>, StorageError> {
    resources_with_uid(backend, MIME_VCARD, "vcf", uid, |data| {
        Contact::from_vcard(&String::from_utf8_lossy(data)).map(|c| c.uid)
    })
    .await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn get_contact
pub async fn get_contact(
    backend: &dyn StorageBackend,
    uid: &str,
) -> Result<Option<Contact>, StorageError> {
    for meta in contact_resources(backend, uid).await? {
        if let Ok((_, data)) = backend.read_resource(meta.id).await {
            if let Some(c) = Contact::from_vcard(&String::from_utf8_lossy(&data)) {
                if c.uid == uid {
                    return Ok(Some(c));
                }
            }
        }
    }
    Ok(None)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn delete_contact
pub async fn delete_contact(backend: &dyn StorageBackend, uid: &str) -> Result<(), StorageError> {
    for meta in contact_resources(backend, uid).await? {
        backend.delete_resource(meta.id).await?;
    }
    Ok(())
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn save_event
pub async fn save_event(
    backend: &dyn StorageBackend,
    mut event: CalendarEvent,
) -> Result<CalendarEvent, StorageError> {
    if event.uid.is_empty() {
        event.uid = new_id().to_string();
    }
    delete_event(backend, &event.uid).await?;
    let ics = event.to_ics();
    let res = Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        event.summary.clone(),
        MIME_ICALENDAR,
        uid_file_name(&event.uid, "ics"),
        ics.len() as u64,
    );
    backend.create_resource(res, ics.into_bytes()).await?;
    Ok(event)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn list_events
pub async fn list_events(backend: &dyn StorageBackend) -> Result<Vec<CalendarEvent>, StorageError> {
    let mut events = Vec::new();
    for (_, data) in resources_with_mime(backend, MIME_ICALENDAR).await? {
        if let Some(e) = CalendarEvent::from_ics(&String::from_utf8_lossy(&data)) {
            events.push(e);
        }
    }
    Ok(events)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn event_resources
async fn event_resources(
    backend: &dyn StorageBackend,
    uid: &str,
) -> Result<Vec<Resource>, StorageError> {
    resources_with_uid(backend, MIME_ICALENDAR, "ics", uid, |data| {
        CalendarEvent::from_ics(&String::from_utf8_lossy(data)).map(|e| e.uid)
    })
    .await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn get_event
pub async fn get_event(
    backend: &dyn StorageBackend,
    uid: &str,
) -> Result<Option<CalendarEvent>, StorageError> {
    for meta in event_resources(backend, uid).await? {
        if let Ok((_, data)) = backend.read_resource(meta.id).await {
            if let Some(e) = CalendarEvent::from_ics(&String::from_utf8_lossy(&data)) {
                if e.uid == uid {
                    return Ok(Some(e));
                }
            }
        }
    }
    Ok(None)
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn delete_event
pub async fn delete_event(backend: &dyn StorageBackend, uid: &str) -> Result<(), StorageError> {
    for meta in event_resources(backend, uid).await? {
        backend.delete_resource(meta.id).await?;
    }
    Ok(())
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn import_todo
pub async fn import_todo(backend: &dyn StorageBackend, ics: &str) -> Result<Note, StorageError> {
    let todo = CalendarTodo::from_ics(ics)
        .ok_or_else(|| StorageError::InvalidInput("no VTODO in input".into()))?;
    let mut note = Note::new("", "");
    todo.apply_to_note(&mut note);
    backend.create_note(note).await
}
```

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

**Code** — complete and verbatim:

```rust
// md:fn import_todos
pub async fn import_todos(
    backend: &dyn StorageBackend,
    ics: &str,
) -> Result<Vec<Note>, StorageError> {
    let todos = CalendarTodo::from_ics_all(ics);
    if todos.is_empty() {
        return Err(StorageError::InvalidInput("no VTODO in input".into()));
    }
    let mut notes = Vec::with_capacity(todos.len());
    for todo in todos {
        let mut note = Note::new("", "");
        todo.apply_to_note(&mut note);
        notes.push(backend.create_note(note).await?);
    }
    Ok(notes)
}
```

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

**Code** — container: members documented as sub-blocks below: fn contact_round_trips_through_vcard, fn vcard_escaping_survives, fn user_vcard_carries_name_and_email, fn event_round_trips_through_ics, fn todo_round_trips_and_marks_completion, fn todo_maps_to_and_from_a_note, fn unfolds_wrapped_lines, fn missing_component_yields_none, fn multi_component_calendar_parses_every_event_and_todo, fn fs, fn contact_save_list_get_delete_over_storage, fn event_round_trips_through_storage, fn import_todo_creates_a_todo_note.

**What it does** — Round-trip fidelity, escaping, folding, multi-component
parsing, the note bridge, and the storage layer end-to-end.

**Dependencies** — `super::*`, `models::Note`, `tempfile`, `tokio`.

**Used by** — CI.

**Repeated context** — none.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
    use crate::models::Note;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::*` — the items under test from the parent module; expects: the parent keeps them at module scope; a rename or a move into a submodule breaks these tests at compile time, which is the intended early signal.
- `crate::models::Note` — the domain type the assertions construct; expects: its public fields and constructor stay usable standalone, without a live store.

**Used by** — every block of `mod tests` in this file: `fn contact_round_trips_through_vcard`, `fn vcard_escaping_survives`, `fn user_vcard_carries_name_and_email`, `fn event_round_trips_through_ics`, `fn todo_round_trips_and_marks_completion`, `fn todo_maps_to_and_from_a_note`, `fn unfolds_wrapped_lines`, `fn missing_component_yields_none`, `fn multi_component_calendar_parses_every_event_and_todo`, `fn fs`, `fn contact_save_list_get_delete_over_storage`, `fn event_round_trips_through_storage`, `fn import_todo_creates_a_todo_note`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn contact_round_trips_through_vcard

**Identification** — unit test; marker
`// md:mod tests > fn contact_round_trips_through_vcard`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn contact_round_trips_through_vcard
    #[test]
    fn contact_round_trips_through_vcard() {
        let c = Contact {
            uid: "abc-123".into(),
            formatted_name: "Ada Lovelace".into(),
            family_name: Some("Lovelace".into()),
            given_name: Some("Ada".into()),
            emails: vec!["ada@example.com".into(), "a@work.com".into()],
            phones: vec!["+1 555 0100".into()],
            org: Some("Analytical Engines; Ltd".into()),
            note: Some("Line1\nLine2, with comma".into()),
            extra: vec!["X-CUSTOM:kept".into()],
        };
        let parsed = Contact::from_vcard(&c.to_vcard()).unwrap();
        assert_eq!(parsed, c, "vCard round-trip must be lossless");
    }
```

**What it does** — A fully-populated `Contact` (multiple emails, an org with
`;`, a multi-line note with a comma, an `X-CUSTOM` extra) survives
`to_vcard → from_vcard` exactly equal.

### fn vcard_escaping_survives

**Identification** — unit test; marker
`// md:mod tests > fn vcard_escaping_survives`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn vcard_escaping_survives
    #[test]
    fn vcard_escaping_survives() {
        let c = Contact {
            formatted_name: "A, B; C\\D".into(),
            ..Default::default()
        };
        let parsed = Contact::from_vcard(&c.to_vcard()).unwrap();
        assert_eq!(parsed.formatted_name, "A, B; C\\D");
    }
```

**What it does** — `A, B; C\D` in `FN` round-trips exactly.

### fn user_vcard_carries_name_and_email

**Identification** — unit test; marker
`// md:mod tests > fn user_vcard_carries_name_and_email`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn user_vcard_carries_name_and_email
    #[test]
    fn user_vcard_carries_name_and_email() {
        let card = user_vcard("Grace Hopper", "grace@navy.mil");
        let parsed = Contact::from_vcard(&card).unwrap();
        assert_eq!(parsed.formatted_name, "Grace Hopper");
        assert_eq!(parsed.emails, vec!["grace@navy.mil".to_string()]);
    }
```

**What it does** — The profile card parses back with the given name and email.

### fn event_round_trips_through_ics

**Identification** — unit test; marker
`// md:mod tests > fn event_round_trips_through_ics`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn event_round_trips_through_ics
    #[test]
    fn event_round_trips_through_ics() {
        let ev = CalendarEvent {
            uid: "evt-1".into(),
            summary: "Launch".into(),
            start: parse_dt("20260101T100000Z"),
            end: parse_dt("20260101T110000Z"),
            location: Some("Pad 39A".into()),
            description: Some("Go for launch".into()),
            extra: vec![],
        };
        let parsed = CalendarEvent::from_ics(&ev.to_ics()).unwrap();
        assert_eq!(parsed, ev);
    }
```

**What it does** — A full `CalendarEvent` survives `to_ics → from_ics`.

### fn todo_round_trips_and_marks_completion

**Identification** — unit test; marker
`// md:mod tests > fn todo_round_trips_and_marks_completion`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn todo_round_trips_and_marks_completion
    #[test]
    fn todo_round_trips_and_marks_completion() {
        let td = CalendarTodo {
            uid: "td-1".into(),
            summary: "Ship it".into(),
            due: parse_dt("20260201T090000Z"),
            completed: parse_dt("20260115T120000Z"),
            description: Some("the thing".into()),
            extra: vec![],
        };
        let ics = td.to_ics();
        assert!(ics.contains("STATUS:COMPLETED"));
        let parsed = CalendarTodo::from_ics(&ics).unwrap();
        assert_eq!(parsed, td);
    }
```

**What it does** — A completed `CalendarTodo` emits `STATUS:COMPLETED` and
round-trips equal.

### fn todo_maps_to_and_from_a_note

**Identification** — unit test; marker
`// md:mod tests > fn todo_maps_to_and_from_a_note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn todo_maps_to_and_from_a_note
    #[test]
    fn todo_maps_to_and_from_a_note() {
        let mut note = Note::new("Buy milk", "2%");
        note.is_todo = true;
        note.todo_due = parse_dt("20260301T000000Z");
        let td = CalendarTodo::from_note(&note);
        assert_eq!(td.summary, "Buy milk");
        assert_eq!(td.uid, note.id.to_string());
        assert_eq!(td.due, note.todo_due);

        let mut target = Note::new("", "");
        td.apply_to_note(&mut target);
        assert!(target.is_todo);
        assert_eq!(target.title, "Buy milk");
        assert_eq!(target.body, "2%");
        assert_eq!(target.todo_due, note.todo_due);
    }
```

**What it does** — `from_note` copies title/uid/due; `apply_to_note` onto a
blank note sets `is_todo`, title, body, due.

### fn unfolds_wrapped_lines

**Identification** — unit test; marker
`// md:mod tests > fn unfolds_wrapped_lines`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn unfolds_wrapped_lines
    #[test]
    fn unfolds_wrapped_lines() {
        let long = "x".repeat(200);
        let ev = CalendarEvent {
            summary: "s".into(),
            description: Some(long.clone()),
            ..Default::default()
        };
        let parsed = CalendarEvent::from_ics(&ev.to_ics()).unwrap();
        assert_eq!(parsed.description, Some(long));
    }
```

**What it does** — A 200-char `DESCRIPTION` (long enough to fold) rejoins on
parse.

### fn missing_component_yields_none

**Identification** — unit test; marker
`// md:mod tests > fn missing_component_yields_none`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn missing_component_yields_none
    #[test]
    fn missing_component_yields_none() {
        assert!(CalendarEvent::from_ics("not a calendar").is_none());
        assert!(Contact::from_vcard("nope").is_none());
        assert!(CalendarEvent::from_ics_all("not a calendar").is_empty());
    }
```

**What it does** — Non-calendar/non-card input → `None`/empty, never a panic.

### fn multi_component_calendar_parses_every_event_and_todo

**Identification** — unit test; marker
`// md:mod tests > fn multi_component_calendar_parses_every_event_and_todo`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn multi_component_calendar_parses_every_event_and_todo
    #[test]
    fn multi_component_calendar_parses_every_event_and_todo() {
        let a = CalendarEvent {
            uid: "e1".into(),
            summary: "First".into(),
            ..Default::default()
        };
        let b = CalendarEvent {
            uid: "e2".into(),
            summary: "Second".into(),
            ..Default::default()
        };
        let td = CalendarTodo {
            uid: "t1".into(),
            summary: "Task".into(),
            ..Default::default()
        };
        let inner = |ics: String, begin: &str, end: &str| -> String {
            let start = ics.find(begin).unwrap();
            let stop = ics.find(end).unwrap() + end.len();
            ics[start..stop].to_string()
        };
        let calendar = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}\r\n{}\r\n{}\r\nEND:VCALENDAR\r\n",
            inner(a.to_ics(), "BEGIN:VEVENT", "END:VEVENT"),
            inner(b.to_ics(), "BEGIN:VEVENT", "END:VEVENT"),
            inner(td.to_ics(), "BEGIN:VTODO", "END:VTODO"),
        );

        let events = CalendarEvent::from_ics_all(&calendar);
        assert_eq!(events.len(), 2, "both VEVENTs import");
        assert_eq!(events[0].uid, "e1");
        assert_eq!(events[1].uid, "e2");
        assert_eq!(CalendarEvent::from_ics(&calendar).unwrap().uid, "e1");

        let todos = CalendarTodo::from_ics_all(&calendar);
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].uid, "t1");
    }
```

**What it does** — Splices two `VEVENT`s and one `VTODO` into a single
`VCALENDAR`: `from_ics_all` yields both events in order plus the todo;
`from_ics` keeps its first-component behaviour.

### fn fs

**Identification** — helper `async fn fs() -> FsBackend`; marker
`// md:mod tests > fn fs`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn fs
    async fn fs() -> crate::storage::fs::FsBackend {
        crate::storage::fs::FsBackend::new(tempfile::tempdir().unwrap().keep())
            .await
            .unwrap()
    }
```

**What it does** — An `FsBackend` in a kept tempdir.

### fn contact_save_list_get_delete_over_storage

**Identification** — tokio test; marker
`// md:mod tests > fn contact_save_list_get_delete_over_storage`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn contact_save_list_get_delete_over_storage
    #[tokio::test]
    async fn contact_save_list_get_delete_over_storage() {
        let be = fs().await;
        let saved = save_contact(
            &be,
            Contact {
                formatted_name: "Ada".into(),
                emails: vec!["a@b.com".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!saved.uid.is_empty(), "a uid is assigned");
        assert_eq!(list_contacts(&be).await.unwrap().len(), 1);

        let mut edit = saved.clone();
        edit.formatted_name = "Ada Lovelace".into();
        save_contact(&be, edit).await.unwrap();
        let list = list_contacts(&be).await.unwrap();
        assert_eq!(list.len(), 1, "upsert by uid replaces");
        assert_eq!(list[0].formatted_name, "Ada Lovelace");

        assert!(get_contact(&be, &saved.uid).await.unwrap().is_some());
        delete_contact(&be, &saved.uid).await.unwrap();
        assert!(list_contacts(&be).await.unwrap().is_empty());
    }
```

**What it does** — Save assigns a UID; list shows one; a re-save with the same
UID **replaces** (still one, updated name); get finds it; delete empties the
list.

### fn event_round_trips_through_storage

**Identification** — tokio test; marker
`// md:mod tests > fn event_round_trips_through_storage`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn event_round_trips_through_storage
    #[tokio::test]
    async fn event_round_trips_through_storage() {
        let be = fs().await;
        let saved = save_event(
            &be,
            CalendarEvent {
                summary: "Launch".into(),
                start: parse_dt("20260101T100000Z"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = get_event(&be, &saved.uid).await.unwrap().unwrap();
        assert_eq!(got.summary, "Launch");
        assert_eq!(got.start, parse_dt("20260101T100000Z"));
    }
```

**What it does** — `save_event` then `get_event` returns the summary and
start.

### fn import_todo_creates_a_todo_note

**Identification** — tokio test; marker
`// md:mod tests > fn import_todo_creates_a_todo_note`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn import_todo_creates_a_todo_note
    #[tokio::test]
    async fn import_todo_creates_a_todo_note() {
        let be = fs().await;
        let ics = CalendarTodo {
            summary: "Task".into(),
            due: parse_dt("20260101T090000Z"),
            ..Default::default()
        }
        .to_ics();
        let note = import_todo(&be, &ics).await.unwrap();
        assert!(note.is_todo);
        assert_eq!(note.title, "Task");
        assert_eq!(note.todo_due, parse_dt("20260101T090000Z"));
    }
```

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
| 1 | `Overview` | `// md:Overview` |
| 2 | `MIME_VCARD` | `// md:MIME_VCARD` |
| 3 | `MIME_ICALENDAR` | `// md:MIME_ICALENDAR` |
| 4 | `fn unfold` | `// md:fn unfold` |
| 5 | `fn fold_line` | `// md:fn fold_line` |
| 6 | `fn escape_text` | `// md:fn escape_text` |
| 7 | `fn unescape_text` | `// md:fn unescape_text` |
| 8 | `fn split_prop` | `// md:fn split_prop` |
| 9 | `fn format_dt` | `// md:fn format_dt` |
| 10 | `fn parse_dt` | `// md:fn parse_dt` |
| 11 | `PRODID` | `// md:PRODID` |
| 12 | `Contact` | `// md:Contact` |
| 13 | `impl Contact` (container) | `// md:impl Contact` |
| 14 | `fn to_vcard` | `// md:impl Contact > fn to_vcard` |
| 15 | `fn from_vcard` | `// md:impl Contact > fn from_vcard` |
| 16 | `fn user_vcard` | `// md:fn user_vcard` |
| 17 | `CalendarEvent` | `// md:CalendarEvent` |
| 18 | `CalendarTodo` | `// md:CalendarTodo` |
| 19 | `impl CalendarEvent` (container) | `// md:impl CalendarEvent` |
| 20 | `fn to_ics` | `// md:impl CalendarEvent > fn to_ics` |
| 21 | `fn from_ics` | `// md:impl CalendarEvent > fn from_ics` |
| 22 | `fn from_ics_all` | `// md:impl CalendarEvent > fn from_ics_all` |
| 23 | `impl CalendarTodo` (container) | `// md:impl CalendarTodo` |
| 24 | `fn to_ics` | `// md:impl CalendarTodo > fn to_ics` |
| 25 | `fn from_ics` | `// md:impl CalendarTodo > fn from_ics` |
| 26 | `fn from_ics_all` | `// md:impl CalendarTodo > fn from_ics_all` |
| 27 | `fn from_note` | `// md:impl CalendarTodo > fn from_note` |
| 28 | `fn apply_to_note` | `// md:impl CalendarTodo > fn apply_to_note` |
| 29 | `fn write_calendar_open` | `// md:fn write_calendar_open` |
| 30 | `fn write_uid_dtstamp` | `// md:fn write_uid_dtstamp` |
| 31 | `fn split_components` | `// md:fn split_components` |
| 32 | `fn parse_component_lines` | `// md:fn parse_component_lines` |
| 33 | `fn resources_with_mime` | `// md:fn resources_with_mime` |
| 34 | `fn resource_metas_with_mime` | `// md:fn resource_metas_with_mime` |
| 35 | `fn uid_file_name` | `// md:fn uid_file_name` |
| 36 | `fn resources_with_uid` | `// md:fn resources_with_uid` |
| 37 | `fn save_contact` | `// md:fn save_contact` |
| 38 | `fn list_contacts` | `// md:fn list_contacts` |
| 39 | `fn contact_resources` | `// md:fn contact_resources` |
| 40 | `fn get_contact` | `// md:fn get_contact` |
| 41 | `fn delete_contact` | `// md:fn delete_contact` |
| 42 | `fn save_event` | `// md:fn save_event` |
| 43 | `fn list_events` | `// md:fn list_events` |
| 44 | `fn event_resources` | `// md:fn event_resources` |
| 45 | `fn get_event` | `// md:fn get_event` |
| 46 | `fn delete_event` | `// md:fn delete_event` |
| 47 | `fn import_todo` | `// md:fn import_todo` |
| 48 | `fn import_todos` | `// md:fn import_todos` |
| 49 | `mod tests` (container) | `// md:mod tests` |
| 50 | `imports` | `// md:mod tests > imports` |
| 51 | `fn contact_round_trips_through_vcard` | `// md:mod tests > fn contact_round_trips_through_vcard` |
| 52 | `fn vcard_escaping_survives` | `// md:mod tests > fn vcard_escaping_survives` |
| 53 | `fn user_vcard_carries_name_and_email` | `// md:mod tests > fn user_vcard_carries_name_and_email` |
| 54 | `fn event_round_trips_through_ics` | `// md:mod tests > fn event_round_trips_through_ics` |
| 55 | `fn todo_round_trips_and_marks_completion` | `// md:mod tests > fn todo_round_trips_and_marks_completion` |
| 56 | `fn todo_maps_to_and_from_a_note` | `// md:mod tests > fn todo_maps_to_and_from_a_note` |
| 57 | `fn unfolds_wrapped_lines` | `// md:mod tests > fn unfolds_wrapped_lines` |
| 58 | `fn missing_component_yields_none` | `// md:mod tests > fn missing_component_yields_none` |
| 59 | `fn multi_component_calendar_parses_every_event_and_todo` | `// md:mod tests > fn multi_component_calendar_parses_every_event_and_todo` |
| 60 | `fn fs` | `// md:mod tests > fn fs` |
| 61 | `fn contact_save_list_get_delete_over_storage` | `// md:mod tests > fn contact_save_list_get_delete_over_storage` |
| 62 | `fn event_round_trips_through_storage` | `// md:mod tests > fn event_round_trips_through_storage` |
| 63 | `fn import_todo_creates_a_todo_note` | `// md:mod tests > fn import_todo_creates_a_todo_note` |
