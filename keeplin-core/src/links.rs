//! Note bookmarks and inter-note links: types and pure parsing.
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
//! A bookmark is an in-note anchor written as a **markdown link whose destination is exactly
//! `###`** — a link that goes nowhere: `[text](### "alias")`. The link **text** becomes the
//! bookmark's `text`; the optional link **title** (in quotes) becomes its `alias`, defaulting
//! to the text when omitted (`[text](###)`); its `number` is its 1-based position among the
//! note's bookmarks. The body is the single source of truth — there is no bookmark API;
//! bookmarks are created, renamed, and removed by editing the note body.
//!
//! # Links
//!
//! Content links are standard markdown links whose destination starts with `#`, e.g.
//! `[text](#notebook1#note3#5)`. The destination is a reference with this grammar:
//!
//! | Form | Meaning |
//! |------|---------|
//! | `#<note>` | note by alias or uuid |
//! | `#<notebook>#<note>` | notebook + note (each alias or uuid) |
//! | `#<notebook>#<note>#<bookmark>` | + bookmark by alias or number |
//!
//! [`parse_link_ref`] is purely structural and reads a two-segment `#a#b` as `notebook#note`.
//! Resolution (in [`crate::linking`]) is smarter: it keeps that reading when `b` is a
//! resolvable note, but otherwise falls back to `note#bookmark`, so a bookmark can be targeted
//! without naming a notebook (`#note3#anchor5`).

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
    /// The link text of the `[text](###)` declaration.
    pub text: String,
    /// Display/reference alias — the link title (`[text](### "alias")`), defaulting to `text`.
    pub alias: String,
}

/// A bookmark parsed from a note body: its link text and the optional inline alias (the link
/// title). The caller assigns the 1-based `number` by order of appearance and defaults the
/// alias to `text` when none is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedBookmark {
    /// The link text.
    pub text: String,
    /// The link title, when present.
    pub alias: Option<String>,
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

/// Compiled regex for a bookmark declaration: a markdown link whose destination is exactly
/// `###` (a link that goes nowhere) — `[text](### "alias")` or `[text](###)`. Group 1 is the
/// link text, group 2 is the optional title (the alias).
fn bookmark_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\[([^\]]*)\]\(\s*###\s*(?:"([^"]*)")?\s*\)"#).unwrap())
}

/// Compiled regex for markdown links whose destination starts with `#`:
/// `[anything](#dest)`, capturing `#dest` (at least one char after the `#`).
fn content_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[^\]]*\]\(\s*(#[^)\s]+)\s*\)").unwrap())
}

/// Extract every `[text](### "alias")` bookmark declaration in `body`, in order of appearance.
/// The 1-based number of each bookmark is its index in the returned vector plus one. Duplicate
/// texts are kept (each occurrence is a distinct bookmark).
pub fn parse_bookmarks(body: &str) -> Vec<DerivedBookmark> {
    bookmark_re()
        .captures_iter(body)
        .map(|c| DerivedBookmark {
            text: c[1].to_string(),
            alias: c.get(2).map(|m| m.as_str().to_string()),
        })
        .collect()
}

/// Extract the raw `#…` destinations of every markdown link in `body`, in order of
/// appearance. Destinations that do not start with `#` are ignored by the regex.
pub fn parse_content_links(body: &str) -> Vec<String> {
    content_link_re()
        .captures_iter(body)
        .map(|c| c[1].to_string())
        // `[text](###)` is a bookmark declaration, not a link destination.
        .filter(|dest| dest != "###")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_two_three_segments() {
        let one = parse_link_ref("#note3").unwrap();
        assert_eq!(one.note, Reference::Alias("note3".into()));
        assert!(one.notebook.is_none() && one.bookmark.is_none());

        let two = parse_link_ref("#notebook1#note3").unwrap();
        assert_eq!(two.notebook, Some(Reference::Alias("notebook1".into())));
        assert_eq!(two.note, Reference::Alias("note3".into()));
        assert!(two.bookmark.is_none());

        let three = parse_link_ref("#notebook1#note3#anchor5").unwrap();
        assert_eq!(three.bookmark, Some(BookmarkRef::Alias("anchor5".into())));

        let numbered = parse_link_ref("#notebook1#note3#5").unwrap();
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
        assert!(parse_link_ref("note3").is_none()); // missing leading '#'
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
    fn extracts_bookmarks_with_and_without_alias_in_order() {
        let body =
            "Intro [Bookmark1](###) mid\n### not a bookmark (heading)\n[Other](### \"Alias\") end";
        let marks = parse_bookmarks(body);
        assert_eq!(
            marks,
            vec![
                DerivedBookmark {
                    text: "Bookmark1".to_string(),
                    alias: None,
                },
                DerivedBookmark {
                    text: "Other".to_string(),
                    alias: Some("Alias".to_string()),
                },
            ]
        );
    }

    #[test]
    fn extracts_content_links_excluding_bookmarks() {
        let body =
            "see [a](#note3) and [b](#notebook1#note3#5), a bookmark [c](###), but not [d](http://x) or [e](#)";
        let links = parse_content_links(body);
        assert_eq!(
            links,
            vec!["#note3".to_string(), "#notebook1#note3#5".to_string()]
        );
    }
}
