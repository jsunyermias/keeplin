// md:Overview

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::links::{Bookmark, NoteLink};

use crate::storage::note_log::VersionVector;

// md:fn parse_cursor
pub(super) fn parse_cursor(token: Option<&str>) -> (String, String) {
    match token.filter(|t| !t.is_empty()) {
        Some(cursor) => match cursor.split_once('|') {
            Some((ts, id)) => (ts.to_owned(), id.to_owned()),
            None => (String::new(), String::new()),
        },
        None => (String::new(), String::new()),
    }
}

// md:fn build_page
pub(super) fn build_page<T, F>(
    mut rows: Vec<T>,
    limit: usize,
    token_fn: F,
) -> (Vec<T>, Option<String>)
where
    F: Fn(&T) -> String,
{
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    let next_token = if has_more {
        rows.last().map(token_fn)
    } else {
        None
    };
    (rows, next_token)
}

// md:fn bookmarks_to_json
pub(super) fn bookmarks_to_json(bookmarks: &[Bookmark]) -> String {
    serde_json::to_string(bookmarks).unwrap_or_else(|_| "[]".to_string())
}

// md:fn links_to_json
pub(super) fn links_to_json(links: &[NoteLink]) -> String {
    serde_json::to_string(links).unwrap_or_else(|_| "[]".to_string())
}

// md:fn json_to_bookmarks
pub(super) fn json_to_bookmarks(s: &str) -> Vec<Bookmark> {
    serde_json::from_str(s).unwrap_or_default()
}

// md:fn json_to_links
pub(super) fn json_to_links(s: &str) -> Vec<NoteLink> {
    serde_json::from_str(s).unwrap_or_default()
}

// md:fn vv_to_json
pub(super) fn vv_to_json(vv: &VersionVector) -> String {
    serde_json::to_string(vv).unwrap_or_else(|_| "{}".to_string())
}

// md:fn json_to_vv
pub(super) fn json_to_vv(s: &str) -> VersionVector {
    serde_json::from_str(s).unwrap_or_default()
}

// md:fn tombstone_data
pub(super) fn tombstone_data(
    deleted_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> String {
    serde_json::json!({
        "deleted_at": deleted_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}

// md:fn assoc_data
pub(super) fn assoc_data(
    tag_id: Uuid,
    updated_at: DateTime<Utc>,
    vv: &VersionVector,
    last_writer: &str,
) -> String {
    serde_json::json!({
        "tag_id": tag_id,
        "updated_at": updated_at,
        "vv": vv,
        "last_writer": last_writer,
    })
    .to_string()
}

// md:fn assoc_from_data
pub(super) fn assoc_from_data(
    data: &serde_json::Value,
    changed_at: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let updated_at = data
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(changed_at);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (updated_at, vv, last_writer)
}

// md:fn tombstone_from_data
pub(super) fn tombstone_from_data(
    data: &serde_json::Value,
    changed_at: DateTime<Utc>,
) -> (DateTime<Utc>, VersionVector, String) {
    let deleted_at = data
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(changed_at);
    let vv = data
        .get("vv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let last_writer = data
        .get("last_writer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (deleted_at, vv, last_writer)
}
