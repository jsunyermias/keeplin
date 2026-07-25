// md:Overview
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// md:Reference
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reference {
    Id(Uuid),
    Alias(String),
}

// md:impl Reference
impl Reference {
    // md:impl Reference > fn parse
    pub fn parse(segment: &str) -> Self {
        match Uuid::parse_str(segment) {
            Ok(id) => Reference::Id(id),
            Err(_) => Reference::Alias(segment.to_string()),
        }
    }
}

// md:BookmarkRef
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkRef {
    Number(u32),
    Alias(String),
}

// md:impl BookmarkRef
impl BookmarkRef {
    // md:impl BookmarkRef > fn parse
    pub fn parse(segment: &str) -> Self {
        match segment.parse::<u32>() {
            Ok(n) if n >= 1 => BookmarkRef::Number(n),
            _ => BookmarkRef::Alias(segment.to_string()),
        }
    }
}

// md:LinkTarget
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notebook: Option<Reference>,
    pub note: Reference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<BookmarkRef>,
}

// md:LinkSource
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkSource {
    Content,
    Manual,
}

// md:Bookmark
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bookmark {
    pub number: u32,
    pub text: String,
    pub alias: String,
}

// md:DerivedBookmark
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedBookmark {
    pub text: String,
    pub alias: Option<String>,
}

// md:NoteLink
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NoteLink {
    pub source: LinkSource,
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_note_id: Option<Uuid>,
}

// md:impl NoteLink
impl NoteLink {
    // md:impl NoteLink > fn from_raw
    pub fn from_raw(raw: &str, source: LinkSource) -> Option<Self> {
        parse_link_ref(raw)?;
        Some(NoteLink {
            source,
            raw: raw.to_string(),
            target_note_id: None,
        })
    }

    // md:impl NoteLink > fn target
    pub fn target(&self) -> Option<LinkTarget> {
        parse_link_ref(&self.raw)
    }
}

// md:fn parse_link_ref
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

// md:fn bookmark_re
fn bookmark_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\[([^\]]*)\]\(\s*###\s*(?:"([^"]*)")?\s*\)"#).unwrap())
}

// md:fn content_link_re
fn content_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[^\]]*\]\(\s*(#[^)\s]+)\s*\)").unwrap())
}

// md:fn parse_bookmarks
pub fn parse_bookmarks(body: &str) -> Vec<DerivedBookmark> {
    bookmark_re()
        .captures_iter(body)
        .map(|c| DerivedBookmark {
            text: c[1].to_string(),
            alias: c.get(2).map(|m| m.as_str().to_string()),
        })
        .collect()
}

// md:fn parse_content_links
pub fn parse_content_links(body: &str) -> Vec<String> {
    content_link_re()
        .captures_iter(body)
        .map(|c| c[1].to_string())
        .filter(|dest| dest != "###")
        .collect()
}

// md:mod tests
#[cfg(test)]
mod tests {
    // md:mod tests > imports
    use super::*;

    // md:mod tests > fn parses_one_two_three_segments
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

    // md:mod tests > fn parses_uuid_segments_as_ids
    #[test]
    fn parses_uuid_segments_as_ids() {
        let id = Uuid::new_v4();
        let nb = Uuid::new_v4();
        let t = parse_link_ref(&format!("#{nb}#{id}")).unwrap();
        assert_eq!(t.notebook, Some(Reference::Id(nb)));
        assert_eq!(t.note, Reference::Id(id));
    }

    // md:mod tests > fn rejects_malformed_refs
    #[test]
    fn rejects_malformed_refs() {
        assert!(parse_link_ref("note3").is_none());
        assert!(parse_link_ref("#").is_none());
        assert!(parse_link_ref("#a##b").is_none());
        assert!(parse_link_ref("#a#b#c#d").is_none());
    }

    // md:mod tests > fn bookmark_ref_zero_is_alias
    #[test]
    fn bookmark_ref_zero_is_alias() {
        assert_eq!(BookmarkRef::parse("0"), BookmarkRef::Alias("0".into()));
        assert_eq!(BookmarkRef::parse("1"), BookmarkRef::Number(1));
    }

    // md:mod tests > fn extracts_bookmarks_with_and_without_alias_in_order
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

    // md:mod tests > fn extracts_content_links_excluding_bookmarks
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
