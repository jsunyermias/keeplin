// md:Overview
mod backend;
pub mod db;
pub mod fs;
pub mod note_log;

pub use backend::{
    EntityVersion, HistoryRepository, NoteRepository, NotebookRepository, NotebookSortProfile,
    ResourceRepository, StorageBackend, SyncBackend, TagRepository, DEFAULT_HISTORY_LIMIT,
};

// md:DEFAULT_PAGE_SIZE
pub const DEFAULT_PAGE_SIZE: u32 = 100;

// md:MAX_PAGE_SIZE
pub const MAX_PAGE_SIZE: u32 = 1000;

// md:fn effective_page_size
pub(crate) fn effective_page_size(page_size: u32) -> u32 {
    if page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        page_size.min(MAX_PAGE_SIZE)
    }
}

// md:trait SortableRfc3339
pub(crate) trait SortableRfc3339 {
    fn to_sortable_rfc3339(&self) -> String;
}

// md:impl SortableRfc3339 for DateTime Utc
impl SortableRfc3339 for chrono::DateTime<chrono::Utc> {
    fn to_sortable_rfc3339(&self) -> String {
        self.to_rfc3339_opts(chrono::SecondsFormat::Nanos, false)
    }
}

// md:mod tests
#[cfg(test)]
mod tests {
    // md:mod tests > imports
    use super::SortableRfc3339;
    use chrono::{DateTime, TimeZone, Utc};

    // md:mod tests > fn effective_page_size_defaults_and_clamps
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

    // md:mod tests > fn sortable_rfc3339_has_fixed_shape
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

    // md:mod tests > fn lexicographic_order_matches_chronological_even_mixed_with_old_format
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

        let mut tagged: Vec<(DateTime<Utc>, String)> = Vec::new();
        for t in &instants {
            tagged.push((*t, t.to_rfc3339()));
            tagged.push((*t, t.to_sortable_rfc3339()));
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
