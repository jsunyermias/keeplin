//! Standards-format interop: vCard (RFC 6350) and iCalendar VTODO/VEVENT (RFC 5545).
//!
//! Keeplin is not a WebDAV/CalDAV server; this module makes it **compatible** with those
//! formats — parsing them in and serialising them back out — so contacts and calendar items
//! move losslessly between Keeplin and other apps. It is pure (no I/O, no storage): the
//! backends and the daemon build on these types.
//!
//! # Scope
//!
//! - [`Contact`] ⇄ a vCard 4.0 card.
//! - [`CalendarEvent`] ⇄ a `VEVENT`.
//! - [`CalendarTodo`] ⇄ a `VTODO`, which additionally maps to/from a Keeplin to-do
//!   [`Note`](crate::models::Note) ([`CalendarTodo::from_note`] / [`CalendarTodo::apply_to_note`]).
//! - [`user_vcard`] renders a profile card for an account (name + email).
//!
//! # Fidelity
//!
//! A pragmatic, widely-interoperable subset of each RFC is modelled explicitly; the rest of a
//! parsed card/component is preserved verbatim in `extra` lines so a round-trip does not drop
//! properties it does not understand.

use chrono::{DateTime, TimeZone, Utc};

use crate::models::{now, Note};

// ── Low-level line handling (shared by vCard and iCalendar) ─────────────────────

/// Unfold RFC 5545/6350 continuation lines: a CRLF (or LF) followed by a single space or tab
/// is a line fold and is removed, rejoining the wrapped value.
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
    // Drop a trailing empty line from a final newline.
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Fold a content line at 75 octets with CRLF + space, per the RFCs. Folding on a char
/// boundary (not mid-UTF-8) keeps the output valid; most consumers are lenient anyway.
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

/// Escape a TEXT value: `\`, `,`, `;` and newlines are backslash-escaped (RFC 5545 §3.3.11 /
/// RFC 6350 §3.4).
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

/// Reverse [`escape_text`].
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

/// Split a content line into its property **name** (upper-cased, params dropped) and its raw
/// value. `SUMMARY;LANGUAGE=en:Hi` → `("SUMMARY", "Hi")`. Returns `None` for a line with no
/// colon.
fn split_prop(line: &str) -> Option<(String, &str)> {
    let colon = line.find(':')?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    // The name ends at the first ';' (start of params), if any.
    let name_end = head.find(';').unwrap_or(head.len());
    Some((head[..name_end].to_ascii_uppercase(), value))
}

/// Format an instant as an RFC 5545 UTC date-time (`YYYYMMDDTHHMMSSZ`).
fn format_dt(dt: DateTime<Utc>) -> String {
    dt.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Parse an RFC 5545 date-time or date. Accepts `YYYYMMDDTHHMMSSZ` (UTC), `YYYYMMDDTHHMMSS`
/// (treated as UTC), and a bare `YYYYMMDD` date (midnight UTC).
fn parse_dt(value: &str) -> Option<DateTime<Utc>> {
    // The `Z` designates UTC but is a literal here (not a `%z` offset), so parse the naive
    // wall-clock and stamp it UTC. A form without `Z` is also read as UTC.
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

/// The Keeplin product id stamped on emitted calendars.
const PRODID: &str = "-//Keeplin//Keeplin//EN";

// ── vCard ───────────────────────────────────────────────────────────────────────

/// A contact, modelling the widely-used subset of vCard 4.0.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contact {
    /// `UID` — a stable identifier (generated on export if empty).
    pub uid: String,
    /// `FN` — the formatted display name (required by the spec).
    pub formatted_name: String,
    /// `N` structured name: (family, given).
    pub family_name: Option<String>,
    pub given_name: Option<String>,
    /// `EMAIL` values, in order.
    pub emails: Vec<String>,
    /// `TEL` values, in order.
    pub phones: Vec<String>,
    /// `ORG` — organisation.
    pub org: Option<String>,
    /// `NOTE` — free text.
    pub note: Option<String>,
    /// Properties this module does not model, kept verbatim for a lossless round-trip.
    pub extra: Vec<String>,
}

impl Contact {
    /// Serialise to a vCard 4.0 card.
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

    /// Parse the first `VCARD` in `input`. Unknown properties are preserved in `extra`.
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
            // A card with no FN still round-trips (FN defaults to empty).
            Some(c)
        } else {
            None
        }
    }
}

/// Render a profile vCard for an account owner from their display name and email.
pub fn user_vcard(display_name: &str, email: &str) -> String {
    Contact {
        formatted_name: display_name.to_string(),
        emails: vec![email.to_string()],
        ..Default::default()
    }
    .to_vcard()
}

// ── iCalendar VEVENT / VTODO ─────────────────────────────────────────────────────

/// A calendar event, modelling the common subset of `VEVENT`.
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

/// A calendar to-do, modelling the common subset of `VTODO`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarTodo {
    pub uid: String,
    pub summary: String,
    pub due: Option<DateTime<Utc>>,
    pub completed: Option<DateTime<Utc>>,
    pub description: Option<String>,
    pub extra: Vec<String>,
}

impl CalendarEvent {
    /// Serialise as a full `VCALENDAR` wrapping one `VEVENT`.
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

    /// Parse the first `VEVENT` found in `input`.
    pub fn from_ics(input: &str) -> Option<CalendarEvent> {
        let mut ev = CalendarEvent::default();
        if parse_component(input, "VEVENT", |name, value, line| match name {
            "UID" => ev.uid = unescape_text(value),
            "SUMMARY" => ev.summary = unescape_text(value),
            "DTSTART" => ev.start = parse_dt(value),
            "DTEND" => ev.end = parse_dt(value),
            "LOCATION" => ev.location = Some(unescape_text(value)),
            "DESCRIPTION" => ev.description = Some(unescape_text(value)),
            "DTSTAMP" => {}
            _ => ev.extra.push(line.to_string()),
        }) {
            Some(ev)
        } else {
            None
        }
    }
}

impl CalendarTodo {
    /// Serialise as a full `VCALENDAR` wrapping one `VTODO`.
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

    /// Parse the first `VTODO` found in `input`.
    pub fn from_ics(input: &str) -> Option<CalendarTodo> {
        let mut td = CalendarTodo::default();
        if parse_component(input, "VTODO", |name, value, line| match name {
            "UID" => td.uid = unescape_text(value),
            "SUMMARY" => td.summary = unescape_text(value),
            "DUE" => td.due = parse_dt(value),
            "COMPLETED" => td.completed = parse_dt(value),
            "DESCRIPTION" => td.description = Some(unescape_text(value)),
            "STATUS" | "PERCENT-COMPLETE" | "DTSTAMP" => {}
            _ => td.extra.push(line.to_string()),
        }) {
            Some(td)
        } else {
            None
        }
    }

    /// Build a `VTODO` view of a Keeplin to-do note (`title`→`SUMMARY`, `body`→`DESCRIPTION`,
    /// `todo_due`→`DUE`, `todo_completed`→`COMPLETED`; the note id becomes the `UID`).
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

    /// Apply this `VTODO` onto `note`, marking it a to-do. The note's id is **not** changed
    /// (the caller owns identity); only the to-do fields, title and body are set.
    pub fn apply_to_note(&self, note: &mut Note) {
        note.title = self.summary.clone();
        note.body = self.description.clone().unwrap_or_default();
        note.is_todo = true;
        note.todo_due = self.due;
        note.todo_completed = self.completed;
    }
}

fn write_calendar_open(out: &mut String) {
    fold_line("BEGIN:VCALENDAR", out);
    fold_line("VERSION:2.0", out);
    fold_line(&format!("PRODID:{PRODID}"), out);
}

/// Write `UID` (generating one when empty) and a `DTSTAMP` of now — both required on a
/// calendar component.
fn write_uid_dtstamp(out: &mut String, uid: &str) {
    let uid = if uid.is_empty() {
        crate::models::new_id().to_string()
    } else {
        uid.to_string()
    };
    fold_line(&format!("UID:{}", escape_text(&uid)), out);
    fold_line(&format!("DTSTAMP:{}", format_dt(now())), out);
}

/// Drive `f(name, value, raw_line)` over the properties of the first `BEGIN:<kind>` …
/// `END:<kind>` component in `input`. Returns whether such a component was found.
fn parse_component(input: &str, kind: &str, mut f: impl FnMut(&str, &str, &str)) -> bool {
    let mut inside = false;
    let mut found = false;
    for line in unfold(input) {
        let (name, value) = match split_prop(&line) {
            Some(p) => p,
            None => continue,
        };
        if name == "BEGIN" && value.eq_ignore_ascii_case(kind) {
            inside = true;
            found = true;
            continue;
        }
        if name == "END" && value.eq_ignore_ascii_case(kind) {
            break;
        }
        if inside {
            f(&name, value, &line);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Note;

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

    #[test]
    fn vcard_escaping_survives() {
        let c = Contact {
            formatted_name: "A, B; C\\D".into(),
            ..Default::default()
        };
        let parsed = Contact::from_vcard(&c.to_vcard()).unwrap();
        assert_eq!(parsed.formatted_name, "A, B; C\\D");
    }

    #[test]
    fn user_vcard_carries_name_and_email() {
        let card = user_vcard("Grace Hopper", "grace@navy.mil");
        let parsed = Contact::from_vcard(&card).unwrap();
        assert_eq!(parsed.formatted_name, "Grace Hopper");
        assert_eq!(parsed.emails, vec!["grace@navy.mil".to_string()]);
    }

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

    #[test]
    fn todo_maps_to_and_from_a_note() {
        let mut note = Note::new("Buy milk", "2%");
        note.is_todo = true;
        note.todo_due = parse_dt("20260301T000000Z");
        let td = CalendarTodo::from_note(&note);
        assert_eq!(td.summary, "Buy milk");
        assert_eq!(td.uid, note.id.to_string());
        assert_eq!(td.due, note.todo_due);

        // Apply a VTODO onto a blank note.
        let mut target = Note::new("", "");
        td.apply_to_note(&mut target);
        assert!(target.is_todo);
        assert_eq!(target.title, "Buy milk");
        assert_eq!(target.body, "2%");
        assert_eq!(target.todo_due, note.todo_due);
    }

    #[test]
    fn unfolds_wrapped_lines() {
        // A DESCRIPTION long enough to fold must rejoin on parse.
        let long = "x".repeat(200);
        let ev = CalendarEvent {
            summary: "s".into(),
            description: Some(long.clone()),
            ..Default::default()
        };
        let parsed = CalendarEvent::from_ics(&ev.to_ics()).unwrap();
        assert_eq!(parsed.description, Some(long));
    }

    #[test]
    fn missing_component_yields_none() {
        assert!(CalendarEvent::from_ics("not a calendar").is_none());
        assert!(Contact::from_vcard("nope").is_none());
    }
}
