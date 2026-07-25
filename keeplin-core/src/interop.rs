// md:Overview
use chrono::{DateTime, TimeZone, Utc};

use crate::error::StorageError;
use crate::models::{new_id, now, Note, Resource, SYSTEM_RESOURCE_NOTE_ID};
use crate::storage::StorageBackend;

// md:MIME_VCARD
pub const MIME_VCARD: &str = "text/vcard";
// md:MIME_ICALENDAR
pub const MIME_ICALENDAR: &str = "text/calendar";

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

// md:fn split_prop
fn split_prop(line: &str) -> Option<(String, &str)> {
    let colon = line.find(':')?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let name_end = head.find(';').unwrap_or(head.len());
    Some((head[..name_end].to_ascii_uppercase(), value))
}

// md:fn format_dt
fn format_dt(dt: DateTime<Utc>) -> String {
    dt.format("%Y%m%dT%H%M%SZ").to_string()
}

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

// md:PRODID
const PRODID: &str = "-//Keeplin//Keeplin//EN";

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

// md:impl Contact
impl Contact {
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
}

// md:fn user_vcard
pub fn user_vcard(display_name: &str, email: &str) -> String {
    Contact {
        formatted_name: display_name.to_string(),
        emails: vec![email.to_string()],
        ..Default::default()
    }
    .to_vcard()
}

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

// md:impl CalendarEvent
impl CalendarEvent {
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

    // md:impl CalendarEvent > fn from_ics
    pub fn from_ics(input: &str) -> Option<CalendarEvent> {
        Self::from_ics_all(input).into_iter().next()
    }

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
}

// md:impl CalendarTodo
impl CalendarTodo {
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

    // md:impl CalendarTodo > fn from_ics
    pub fn from_ics(input: &str) -> Option<CalendarTodo> {
        Self::from_ics_all(input).into_iter().next()
    }

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

    // md:impl CalendarTodo > fn apply_to_note
    pub fn apply_to_note(&self, note: &mut Note) {
        note.title = self.summary.clone();
        note.body = self.description.clone().unwrap_or_default();
        note.is_todo = true;
        note.todo_due = self.due;
        note.todo_completed = self.completed;
    }
}

// md:fn write_calendar_open
fn write_calendar_open(out: &mut String) {
    fold_line("BEGIN:VCALENDAR", out);
    fold_line("VERSION:2.0", out);
    fold_line(&format!("PRODID:{PRODID}"), out);
}

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

// md:fn parse_component_lines
fn parse_component_lines(lines: &[String], mut f: impl FnMut(&str, &str, &str)) {
    for line in lines {
        if let Some((name, value)) = split_prop(line) {
            f(&name, value, line);
        }
    }
}

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

// md:fn uid_file_name
fn uid_file_name(uid: &str, ext: &str) -> String {
    format!("{uid}.{ext}")
}

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

// md:fn delete_contact
pub async fn delete_contact(backend: &dyn StorageBackend, uid: &str) -> Result<(), StorageError> {
    for meta in contact_resources(backend, uid).await? {
        backend.delete_resource(meta.id).await?;
    }
    Ok(())
}

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

// md:fn delete_event
pub async fn delete_event(backend: &dyn StorageBackend, uid: &str) -> Result<(), StorageError> {
    for meta in event_resources(backend, uid).await? {
        backend.delete_resource(meta.id).await?;
    }
    Ok(())
}

// md:fn import_todo
pub async fn import_todo(backend: &dyn StorageBackend, ics: &str) -> Result<Note, StorageError> {
    let todo = CalendarTodo::from_ics(ics)
        .ok_or_else(|| StorageError::InvalidInput("no VTODO in input".into()))?;
    let mut note = Note::new("", "");
    todo.apply_to_note(&mut note);
    backend.create_note(note).await
}

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

// md:mod tests
#[cfg(test)]
mod tests {
    // md:mod tests > imports
    use super::*;
    use crate::models::Note;

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

    // md:mod tests > fn user_vcard_carries_name_and_email
    #[test]
    fn user_vcard_carries_name_and_email() {
        let card = user_vcard("Grace Hopper", "grace@navy.mil");
        let parsed = Contact::from_vcard(&card).unwrap();
        assert_eq!(parsed.formatted_name, "Grace Hopper");
        assert_eq!(parsed.emails, vec!["grace@navy.mil".to_string()]);
    }

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

    // md:mod tests > fn missing_component_yields_none
    #[test]
    fn missing_component_yields_none() {
        assert!(CalendarEvent::from_ics("not a calendar").is_none());
        assert!(Contact::from_vcard("nope").is_none());
        assert!(CalendarEvent::from_ics_all("not a calendar").is_empty());
    }

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

    // md:mod tests > fn fs
    async fn fs() -> crate::storage::fs::FsBackend {
        crate::storage::fs::FsBackend::new(tempfile::tempdir().unwrap().keep())
            .await
            .unwrap()
    }

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
}
