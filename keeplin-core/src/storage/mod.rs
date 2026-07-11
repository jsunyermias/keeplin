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
    EntityVersion, HistoryRepository, NoteRepository, NotebookRepository, NotebookSortProfile,
    ResourceRepository, StorageBackend, SyncBackend, TagRepository, DEFAULT_HISTORY_LIMIT,
};

/// Page size used when a list call passes `page_size = 0`.
pub const DEFAULT_PAGE_SIZE: u32 = 100;

/// Hard upper bound applied to every list call's `page_size`.
///
/// `page_size` arrives from the network (gRPC/REST) as an arbitrary `u32`; without a cap a
/// single request for `u32::MAX` rows would make the server materialize the entire store in
/// one response. Requests above the cap are silently clamped rather than rejected — the
/// cursor in the reply lets a well-behaved client keep paging.
pub const MAX_PAGE_SIZE: u32 = 1000;

/// Resolve a caller-supplied `page_size` to the limit actually used: `0` means
/// [`DEFAULT_PAGE_SIZE`], anything above [`MAX_PAGE_SIZE`] is clamped down to it.
pub(crate) fn effective_page_size(page_size: u32) -> u32 {
    if page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        page_size.min(MAX_PAGE_SIZE)
    }
}

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
    fn effective_page_size_defaults_and_clamps() {
        assert_eq!(super::effective_page_size(0), super::DEFAULT_PAGE_SIZE);
        assert_eq!(super::effective_page_size(7), 7);
        assert_eq!(
            super::effective_page_size(super::MAX_PAGE_SIZE),
            super::MAX_PAGE_SIZE
        );
        assert_eq!(super::effective_page_size(u32::MAX), super::MAX_PAGE_SIZE);
    }

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
