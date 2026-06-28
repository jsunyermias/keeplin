//! Note bookmarks (marcadores) and inter-note links (enlaces): types and pure parsing.
//!
//! This module is intentionally I/O-free so the grammar can be unit-tested in isolation.
//! It defines the persisted [`Bookmark`] / [`NoteLink`] types that live as fields on
//! [`crate::models::Note`], the parsed [`LinkTarget`] grammar, and the pure functions that
//! extract bookmarks and content links from a markdown body. Resolving an alias/uuid
//! reference to a concrete note (which needs store access) lives in
//! [`crate::linking::LinkingBackend`].
//!
//! # Bookmarks
//!
//! A bookmark is an in-note anchor written as a **triple-hash token** in the body, e.g.
//! `###Marcador1` (a hashtag with three `#`). Its `text` is the marked word (`Marcador1`),
//! its `alias` defaults to that text but is editable, and its `number` is its 1-based
//! position among the note's bookmarks. The token must sit at a word boundary, have exactly
//! three `#`, and be followed by a non-space, non-`#` character — so it never collides with
//! a markdown `### ` heading (space after the hashes) or a longer `####` run.
//!
//! # Links
//!
//! Content links are standard markdown links whose destination starts with `#`, e.g.
//! `[texto](#libreta1#nota3#5)`. The destination is a reference with this grammar:
//!
//! | Form | Meaning |
//! |------|---------|
//! | `#<note>` | note by alias or uuid |
//! | `#<notebook>#<note>` | notebook + note (each alias or uuid) |
//! | `#<notebook>#<note>#<bookmark>` | + bookmark by alias or number |
//!
//! The two-segment form is always `notebook#note`; a bookmark target therefore requires the
//! full three-segment form.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A reference to a note or notebook segment: either a concrete UUID or a human alias.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reference {
    /// A concrete entity UUID (the segment parsed as a valid UUID).
    Id(Uuid),
    /// A human-assigned alias (any segment that is not a valid UUID).
    Alias(String),
}

impl Reference {
    /// Parse one reference segment: a valid UUID becomes [`Reference::Id`], anything else
    /// an [`Reference::Alias`].
    pub fn parse(segment: &str) -> Self {
        match Uuid::parse_str(segment) {
            Ok(id) => Reference::Id(id),
            Err(_) => Reference::Alias(segment.to_string()),
        }
    }
}

/// The optional third segment of a link: a bookmark by 1-based number or by alias.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkRef {
    /// A bookmark addressed by its 1-based position in the note.
    Number(u32),
    /// A bookmark addressed by its (default or edited) alias.
    Alias(String),
}

impl BookmarkRef {
    /// Parse a bookmark segment: an unsigned integer becomes [`BookmarkRef::Number`],
    /// anything else a [`BookmarkRef::Alias`]. `0` is treated as an alias because bookmark
    /// numbering is 1-based.
    pub fn parse(segment: &str) -> Self {
        match segment.parse::<u32>() {
            Ok(n) if n >= 1 => BookmarkRef::Number(n),
            _ => BookmarkRef::Alias(segment.to_string()),
        }
    }
}

/// A fully parsed `#…` link reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkTarget {
    /// The notebook scope, when the reference has two or more segments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notebook: Option<Reference>,
    /// The target note (always present).
    pub note: Reference,
    /// The target bookmark within the note, when the reference has three segments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<BookmarkRef>,
}

/// Where a [`NoteLink`] came from: parsed out of the body, or added explicitly via the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkSource {
    /// Derived from a markdown link in the note body. Recomputed on every write.
    Content,
    /// Added directly between notes via the API; not present in the body and preserved
    /// across body edits.
    Manual,
}

/// A bookmark anchor within a note.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bookmark {
    /// 1-based position among the note's bookmarks, in order of appearance in the body.
    pub number: u32,
    /// The marked text from the `###text` token. Stable identity for carrying alias edits
    /// across body changes.
    pub text: String,
    /// Display/reference alias. Defaults to `text`; editable via the API.
    pub alias: String,
}

/// A link from one note to a target note (optionally scoped by notebook and bookmark).
///
/// Only the literal `raw` reference, its `source`, and a best-effort resolved
/// `target_note_id` are persisted; the parsed [`LinkTarget`] is derived on demand from
/// `raw` via [`NoteLink::target`]. Keeping the persisted form to a single human string plus
/// a UUID makes at-rest encryption straightforward (only `raw` needs encrypting; the UUID
/// stays plaintext, like `notebook_id`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NoteLink {
    /// Content-derived or manually added.
    pub source: LinkSource,
    /// The literal `#…` reference string, exactly as parsed from the body or supplied.
    pub raw: String,
    /// Best-effort resolution snapshot: the UUID of the target note at write time, or
    /// `None` when the reference could not be resolved (e.g. the target did not exist yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_note_id: Option<Uuid>,
}

impl NoteLink {
    /// Build a [`NoteLink`] from a raw `#…` reference and its source. Returns `None` when
    /// `raw` is not a valid reference.
    pub fn from_raw(raw: &str, source: LinkSource) -> Option<Self> {
        parse_link_ref(raw)?;
        Some(NoteLink {
            source,
            raw: raw.to_string(),
            target_note_id: None,
        })
    }

    /// Parse the persisted `raw` reference into its [`LinkTarget`] components.
    pub fn target(&self) -> Option<LinkTarget> {
        parse_link_ref(&self.raw)
    }
}

/// Parse a `#…` reference string into a [`LinkTarget`].
///
/// Accepts one, two, or three non-empty `#`-separated segments. Returns `None` when the
/// string does not start with `#`, has an empty segment, or has more than three segments.
pub fn parse_link_ref(s: &str) -> Option<LinkTarget> {
    let body = s.strip_prefix('#')?;
    let segments: Vec<&str> = body.split('#').collect();
    if segments.iter().any(|seg| seg.is_empty()) {
        return None;
    }
    match segments.as_slice() {
        [note] => Some(LinkTarget {
            notebook: None,
            note: Reference::parse(note),
            bookmark: None,
        }),
        [notebook, note] => Some(LinkTarget {
            notebook: Some(Reference::parse(notebook)),
            note: Reference::parse(note),
            bookmark: None,
        }),
        [notebook, note, bookmark] => Some(LinkTarget {
            notebook: Some(Reference::parse(notebook)),
            note: Reference::parse(note),
            bookmark: Some(BookmarkRef::parse(bookmark)),
        }),
        _ => None,
    }
}

/// Compiled regex for `###text` bookmark tokens. Anchored at a word boundary (start or
/// whitespace), exactly three `#`, then a first char that is neither whitespace nor `#`
/// (so `### heading` and `####run` are excluded), then the rest of the non-whitespace run.
fn bookmark_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)###([^\s#]\S*)").unwrap())
}

/// Compiled regex for markdown links whose destination starts with `#`:
/// `[anything](#dest)`, capturing `#dest` (at least one char after the `#`).
fn content_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[^\]]*\]\(\s*(#[^)\s]+)\s*\)").unwrap())
}

/// Extract the marked texts of every `###text` bookmark token in `body`, in order of
/// appearance. The 1-based number of each bookmark is its index in the returned vector
/// plus one. Duplicate texts are kept (each occurrence is a distinct bookmark).
pub fn parse_bookmarks(body: &str) -> Vec<String> {
    bookmark_re()
        .captures_iter(body)
        .map(|c| c[1].to_string())
        .collect()
}

/// Extract the raw `#…` destinations of every markdown link in `body`, in order of
/// appearance. Destinations that do not start with `#` are ignored by the regex.
pub fn parse_content_links(body: &str) -> Vec<String> {
    content_link_re()
        .captures_iter(body)
        .map(|c| c[1].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_two_three_segments() {
        let one = parse_link_ref("#nota3").unwrap();
        assert_eq!(one.note, Reference::Alias("nota3".into()));
        assert!(one.notebook.is_none() && one.bookmark.is_none());

        let two = parse_link_ref("#libreta1#nota3").unwrap();
        assert_eq!(two.notebook, Some(Reference::Alias("libreta1".into())));
        assert_eq!(two.note, Reference::Alias("nota3".into()));
        assert!(two.bookmark.is_none());

        let three = parse_link_ref("#libreta1#nota3#marcador5").unwrap();
        assert_eq!(three.bookmark, Some(BookmarkRef::Alias("marcador5".into())));

        let numbered = parse_link_ref("#libreta1#nota3#5").unwrap();
        assert_eq!(numbered.bookmark, Some(BookmarkRef::Number(5)));
    }

    #[test]
    fn parses_uuid_segments_as_ids() {
        let id = Uuid::new_v4();
        let nb = Uuid::new_v4();
        let t = parse_link_ref(&format!("#{nb}#{id}")).unwrap();
        assert_eq!(t.notebook, Some(Reference::Id(nb)));
        assert_eq!(t.note, Reference::Id(id));
    }

    #[test]
    fn rejects_malformed_refs() {
        assert!(parse_link_ref("nota3").is_none()); // missing leading '#'
        assert!(parse_link_ref("#").is_none()); // empty single segment
        assert!(parse_link_ref("#a##b").is_none()); // empty middle segment
        assert!(parse_link_ref("#a#b#c#d").is_none()); // too many segments
    }

    #[test]
    fn bookmark_ref_zero_is_alias() {
        assert_eq!(BookmarkRef::parse("0"), BookmarkRef::Alias("0".into()));
        assert_eq!(BookmarkRef::parse("1"), BookmarkRef::Number(1));
    }

    #[test]
    fn extracts_bookmarks_in_order_excluding_headings() {
        let body = "Intro ###Marcador1 mid\n### Not a bookmark (heading)\nmore ###Otro y ####nope";
        let marks = parse_bookmarks(body);
        assert_eq!(marks, vec!["Marcador1".to_string(), "Otro".to_string()]);
    }

    #[test]
    fn extracts_content_links() {
        let body = "see [a](#nota3) and [b](#libreta1#nota3#5) but not [c](http://x) or [d](#)";
        let links = parse_content_links(body);
        assert_eq!(
            links,
            vec!["#nota3".to_string(), "#libreta1#nota3#5".to_string()]
        );
    }
}
