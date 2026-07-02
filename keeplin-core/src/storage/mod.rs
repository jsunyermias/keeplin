//! Storage layer for Keeplin.
//!
//! This module provides the [`StorageBackend`] trait that every storage implementation
//! must satisfy, plus two concrete implementations:
//!
//! - [`fs::FsBackend`] — stores data as JSON files on the local filesystem and uses
//!   per-device NDJSON change logs that Syncthing (or any compatible tool) can replicate
//!   across devices.
//! - [`db::DbBackend`] — stores data in a local LibSQL (SQLite-compatible) database and
//!   synchronises with a central server over a WebSocket connection.

mod backend;
pub mod db;
pub mod fs;
pub mod note_log;

pub use backend::{
    NoteRepository, NotebookRepository, ResourceRepository, StorageBackend, SyncBackend,
    TagRepository,
};

/// Fixed-precision RFC 3339 for timestamps that are **compared as text**.
///
/// The backends store timestamps as RFC 3339 TEXT and order them lexicographically —
/// SQLite `WHERE created_at > ?` / `ORDER BY`, and the `"<ts>|<id>"` keyset cursors.
/// Lexicographic order only matches chronological order when every value has the same
/// shape, but `DateTime::to_rfc3339()` emits a *variable* number of fractional digits
/// (3/6/9, whatever the instant needs — e.g. 6 on platforms with microsecond clocks,
/// 9 with nanosecond clocks). Two representations of comparable instants can then
/// order incorrectly, and the `created_at = cursor` equality branch of keyset
/// pagination silently fails across precisions.
///
/// [`to_sortable_rfc3339`](SortableRfc3339::to_sortable_rfc3339) pins the shape:
/// always 9 fractional digits and the `+00:00` offset, so equal instants are equal
/// strings and lexicographic = chronological. Rows written before this existed keep
/// their variable-precision text; ordering against them stays chronologically
/// consistent (the shorter fraction sorts exactly where its value belongs), only their
/// cursor-equality match remains best-effort — the same situation mixed-precision
/// writers were already in.
pub(crate) trait SortableRfc3339 {
    /// Format as RFC 3339 with exactly nine fractional digits and a `+00:00` offset.
    fn to_sortable_rfc3339(&self) -> String;
}

impl SortableRfc3339 for chrono::DateTime<chrono::Utc> {
    fn to_sortable_rfc3339(&self) -> String {
        self.to_rfc3339_opts(chrono::SecondsFormat::Nanos, false)
    }
}

#[cfg(test)]
mod tests {
    use super::SortableRfc3339;
    use chrono::{DateTime, TimeZone, Utc};

    #[test]
    fn sortable_rfc3339_has_fixed_shape() {
        let second_aligned = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let s = second_aligned.to_sortable_rfc3339();
        assert!(s.ends_with("+00:00"), "offset form is kept: {s}");
        let frac = s.split('.').nth(1).unwrap();
        assert_eq!(
            &frac[..9],
            "000000000",
            "always nine fractional digits: {s}"
        );
    }

    /// Lexicographic order must equal chronological order — including against strings
    /// written by the old variable-precision `to_rfc3339()` (0, 3, 6, or 9 digits).
    #[test]
    fn lexicographic_order_matches_chronological_even_mixed_with_old_format() {
        let instants: Vec<DateTime<Utc>> = [
            (100, 0),
            (100, 500_000_000),
            (100, 500_000_001),
            (100, 999_999_999),
            (101, 0),
            (101, 123_456_000),
        ]
        .iter()
        .map(|&(s, n)| Utc.timestamp_opt(s, n).unwrap())
        .collect();

        // Old- and new-format strings for every instant, tagged with the instant.
        let mut tagged: Vec<(DateTime<Utc>, String)> = Vec::new();
        for t in &instants {
            tagged.push((*t, t.to_rfc3339())); // variable precision (legacy rows)
            tagged.push((*t, t.to_sortable_rfc3339())); // fixed precision (new rows)
        }
        let mut by_string = tagged.clone();
        by_string.sort_by(|a, b| a.1.cmp(&b.1));
        let mut by_time = tagged;
        by_time.sort_by_key(|(t, _)| *t);
        assert_eq!(
            by_string.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            by_time.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            "string order must never contradict time order"
        );
    }
}
