// md:Overview
use thiserror::Error;

use crate::error::StorageError;

// md:MAX_LINE_BYTES
pub const MAX_LINE_BYTES: usize = 1 << 12;

// md:MAX_LINES_PER_NOTE
pub const MAX_LINES_PER_NOTE: usize = 1 << 16;

// md:MAX_NOTES_PER_NOTEBOOK
pub const MAX_NOTES_PER_NOTEBOOK: usize = 1 << 24;

// md:CODE_LINE_TOO_LONG
pub const CODE_LINE_TOO_LONG: &str = "too_long";

// md:CODE_TOO_MANY_LINES
pub const CODE_TOO_MANY_LINES: &str = "too_many_lines";

// md:CODE_NOTEBOOK_FULL
pub const CODE_NOTEBOOK_FULL: &str = "notebook_full";

// md:LimitViolation
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LimitViolation {
    #[error("line of {bytes} bytes exceeds the format limit of {MAX_LINE_BYTES} bytes")]
    LineTooLong { bytes: usize },

    #[error("note of {lines} lines exceeds the format limit of {MAX_LINES_PER_NOTE} lines")]
    TooManyLines { lines: usize },

    #[error(
        "notebook already holds {notes} notes, the format limit of {MAX_NOTES_PER_NOTEBOOK} notes"
    )]
    NotebookFull { notes: usize },
}

// md:impl LimitViolation
impl LimitViolation {
    // md:impl LimitViolation > fn code
    pub fn code(&self) -> &'static str {
        match self {
            LimitViolation::LineTooLong { .. } => CODE_LINE_TOO_LONG,
            LimitViolation::TooManyLines { .. } => CODE_TOO_MANY_LINES,
            LimitViolation::NotebookFull { .. } => CODE_NOTEBOOK_FULL,
        }
    }
}

// md:impl From LimitViolation for StorageError
impl From<LimitViolation> for StorageError {
    fn from(violation: LimitViolation) -> Self {
        StorageError::TooLarge(violation.to_string())
    }
}

// md:fn is_limit_code
pub fn is_limit_code(code: &str) -> bool {
    matches!(
        code,
        CODE_LINE_TOO_LONG | CODE_TOO_MANY_LINES | CODE_NOTEBOOK_FULL
    )
}

// md:fn check_line
pub fn check_line(content: &str) -> Result<(), LimitViolation> {
    if content.len() > MAX_LINE_BYTES {
        return Err(LimitViolation::LineTooLong {
            bytes: content.len(),
        });
    }
    Ok(())
}

// md:fn check_line_count
pub fn check_line_count(lines: usize) -> Result<(), LimitViolation> {
    if lines > MAX_LINES_PER_NOTE {
        return Err(LimitViolation::TooManyLines { lines });
    }
    Ok(())
}

// md:fn line_count
pub fn line_count(body: &str) -> usize {
    if body.is_empty() {
        0
    } else {
        body.split('\n').count()
    }
}

// md:fn check_body
pub fn check_body(body: &str) -> Result<(), LimitViolation> {
    if body.is_empty() {
        return Ok(());
    }
    let mut lines = 0usize;
    for line in body.split('\n') {
        lines += 1;
        check_line(line)?;
    }
    check_line_count(lines)
}

// md:fn check_notebook_capacity
pub fn check_notebook_capacity(live_notes: usize) -> Result<(), LimitViolation> {
    if live_notes >= MAX_NOTES_PER_NOTEBOOK {
        return Err(LimitViolation::NotebookFull { notes: live_notes });
    }
    Ok(())
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn the_three_limits_are_exact_powers_of_two
    #[test]
    fn the_three_limits_are_exact_powers_of_two() {
        assert_eq!(MAX_LINE_BYTES, 4096);
        assert_eq!(MAX_LINES_PER_NOTE, 65_536);
        assert_eq!(MAX_NOTES_PER_NOTEBOOK, 16_777_216);
        assert!(MAX_LINE_BYTES.is_power_of_two());
        assert!(MAX_LINES_PER_NOTE.is_power_of_two());
        assert!(MAX_NOTES_PER_NOTEBOOK.is_power_of_two());
    }

    // md:mod tests > fn line_length_is_counted_in_utf8_bytes
    #[test]
    fn line_length_is_counted_in_utf8_bytes() {
        assert!(check_line(&"a".repeat(MAX_LINE_BYTES)).is_ok());
        assert_eq!(
            check_line(&"a".repeat(MAX_LINE_BYTES + 1)),
            Err(LimitViolation::LineTooLong {
                bytes: MAX_LINE_BYTES + 1
            })
        );
        let two_byte_chars = "é".repeat(MAX_LINE_BYTES / 2);
        assert_eq!(two_byte_chars.chars().count(), MAX_LINE_BYTES / 2);
        assert!(check_line(&two_byte_chars).is_ok());
        let one_char_over = "é".repeat(MAX_LINE_BYTES / 2 + 1);
        assert_eq!(
            check_line(&one_char_over),
            Err(LimitViolation::LineTooLong {
                bytes: MAX_LINE_BYTES + 2
            })
        );
    }

    // md:mod tests > fn line_count_boundary_accepts_the_limit_and_rejects_one_more
    #[test]
    fn line_count_boundary_accepts_the_limit_and_rejects_one_more() {
        assert!(check_line_count(MAX_LINES_PER_NOTE).is_ok());
        assert_eq!(
            check_line_count(MAX_LINES_PER_NOTE + 1),
            Err(LimitViolation::TooManyLines {
                lines: MAX_LINES_PER_NOTE + 1
            })
        );
    }

    // md:mod tests > fn line_count_matches_the_materialised_body
    #[test]
    fn line_count_matches_the_materialised_body() {
        assert_eq!(line_count(""), 0);
        assert_eq!(line_count("a"), 1);
        assert_eq!(line_count("a\nb"), 2);
        assert_eq!(line_count("a\n"), 2);
    }

    // md:mod tests > fn check_body_enforces_both_line_limits
    #[test]
    fn check_body_enforces_both_line_limits() {
        assert!(check_body("").is_ok());
        let at_line_limit = "x\n".repeat(MAX_LINES_PER_NOTE - 1) + "x";
        assert_eq!(line_count(&at_line_limit), MAX_LINES_PER_NOTE);
        assert!(check_body(&at_line_limit).is_ok());
        let over_line_limit = "x\n".repeat(MAX_LINES_PER_NOTE) + "x";
        assert_eq!(line_count(&over_line_limit), MAX_LINES_PER_NOTE + 1);
        assert_eq!(
            check_body(&over_line_limit),
            Err(LimitViolation::TooManyLines {
                lines: MAX_LINES_PER_NOTE + 1
            })
        );
        let long_line = format!("ok\n{}", "a".repeat(MAX_LINE_BYTES + 1));
        assert_eq!(
            check_body(&long_line),
            Err(LimitViolation::LineTooLong {
                bytes: MAX_LINE_BYTES + 1
            })
        );
    }

    // md:mod tests > fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap
    #[test]
    fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap() {
        assert!(check_notebook_capacity(0).is_ok());
        assert!(check_notebook_capacity(MAX_NOTES_PER_NOTEBOOK - 1).is_ok());
        assert_eq!(
            check_notebook_capacity(MAX_NOTES_PER_NOTEBOOK),
            Err(LimitViolation::NotebookFull {
                notes: MAX_NOTES_PER_NOTEBOOK
            })
        );
    }

    // md:mod tests > fn every_violation_maps_to_its_wire_code_and_to_too_large
    #[test]
    fn every_violation_maps_to_its_wire_code_and_to_too_large() {
        let violations = [
            LimitViolation::LineTooLong { bytes: 1 },
            LimitViolation::TooManyLines { lines: 1 },
            LimitViolation::NotebookFull { notes: 1 },
        ];
        for violation in violations {
            assert!(is_limit_code(violation.code()));
            let mapped: StorageError = violation.clone().into();
            assert!(matches!(mapped, StorageError::TooLarge(_)));
            assert_eq!(mapped.to_string(), format!("Too large: {violation}"));
        }
        assert!(!is_limit_code("bad_content"));
    }
}
