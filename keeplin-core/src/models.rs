// md:Overview
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::links::{Bookmark, NoteLink};
use crate::storage::note_log::VersionVector;

// md:fn new_id
pub fn new_id() -> Uuid {
    Uuid::new_v4()
}

// md:fn now
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

// md:Note
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    #[serde(default = "Uuid::nil", deserialize_with = "de_notebook_id")]
    pub notebook_id: Uuid,
    pub is_todo: bool,
    pub todo_due: Option<DateTime<Utc>>,
    pub todo_completed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub links: Vec<NoteLink>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_starred: bool,
    #[serde(default)]
    pub sort_key: u32,
}

// md:fn de_notebook_id
fn de_notebook_id<'de, D>(deserializer: D) -> Result<Uuid, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Uuid>::deserialize(deserializer)?.unwrap_or_else(Uuid::nil))
}

// md:impl Note
impl Note {
    // md:impl Note > DEFAULT_SORT_KEY
    pub const DEFAULT_SORT_KEY: u32 = 1000;

    // md:impl Note > fn new
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            body: body.into(),
            notebook_id: Uuid::nil(),
            is_todo: false,
            todo_due: None,
            todo_completed: None,
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            alias: None,
            bookmarks: Vec::new(),
            links: Vec::new(),
            vv: VersionVector::new(),
            last_writer: String::new(),
            is_pinned: false,
            is_starred: false,
            sort_key: 0,
        }
    }

    // md:impl Note > fn effective_sort_key
    pub fn effective_sort_key(&self) -> u32 {
        if self.sort_key == 0 {
            Self::DEFAULT_SORT_KEY
        } else {
            self.sort_key
        }
    }
}

// md:Notebook
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Notebook {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
}

// md:impl Notebook
impl Notebook {
    // md:impl Notebook > fn new
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            alias: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        }
    }
}

// md:Tag
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Tag {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
    #[serde(default)]
    pub system: bool,
}

// md:impl Tag
impl Tag {
    // md:impl Tag > fn new
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
            system: false,
        }
    }
}

// md:NoteTag
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NoteTag {
    pub note_id: Uuid,
    pub tag_id: Uuid,
}

// md:Resource
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Resource {
    pub id: Uuid,
    pub title: String,
    pub mime_type: String,
    pub file_name: String,
    pub size: u64,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub vv: VersionVector,
    #[serde(default)]
    pub last_writer: String,
}

// md:impl Resource
impl Resource {
    // md:impl Resource > fn new
    pub fn new(
        title: impl Into<String>,
        mime_type: impl Into<String>,
        file_name: impl Into<String>,
        size: u64,
    ) -> Self {
        Self {
            id: new_id(),
            title: title.into(),
            mime_type: mime_type.into(),
            file_name: file_name.into(),
            size,
            created_at: now(),
            deleted_at: None,
            vv: VersionVector::new(),
            last_writer: String::new(),
        }
    }
}

// md:Change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    #[serde(alias = "create")]
    NoteCreate {
        note: Note,
    },
    #[serde(alias = "update")]
    NoteUpdate {
        note: Note,
    },
    #[serde(alias = "delete")]
    NoteDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    NotebookCreate {
        notebook: Notebook,
    },
    NotebookUpdate {
        notebook: Notebook,
    },
    NotebookDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    TagCreate {
        tag: Tag,
    },
    TagUpdate {
        tag: Tag,
    },
    TagDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    NoteTagAdd {
        note_id: Uuid,
        tag_id: Uuid,
        #[serde(default = "now")]
        updated_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    NoteTagRemove {
        note_id: Uuid,
        tag_id: Uuid,
        #[serde(default = "now")]
        updated_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
    ResourceCreate {
        resource: Resource,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        data: Option<Vec<u8>>,
    },
    ResourceDelete {
        id: Uuid,
        #[serde(default = "now")]
        deleted_at: DateTime<Utc>,
        #[serde(default)]
        vv: VersionVector,
        #[serde(default)]
        last_writer: String,
    },
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn pre_ordering_note_json_lands_in_the_inbox_with_defaults
    #[test]
    fn pre_ordering_note_json_lands_in_the_inbox_with_defaults() {
        let old = r#"{
            "id": "6f2a5b1c-9d5c-4c3a-8f21-3b1a2c4d5e6f",
            "title": "t", "body": "b",
            "notebook_id": null,
            "is_todo": false, "todo_due": null, "todo_completed": null,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "deleted_at": null
        }"#;
        let note: Note = serde_json::from_str(old).unwrap();
        assert_eq!(note.notebook_id, Uuid::nil(), "null lands in the Inbox");
        assert!(!note.is_pinned);
        assert!(!note.is_starred);
        assert_eq!(note.sort_key, 0);
        assert_eq!(note.effective_sort_key(), Note::DEFAULT_SORT_KEY);
    }

    // md:mod tests > fn pre_ordering_note_msgpack_round_trips
    #[test]
    fn pre_ordering_note_msgpack_round_trips() {
        #[derive(serde::Serialize)]
        struct OldNote<'a> {
            id: Uuid,
            title: &'a str,
            body: &'a str,
            notebook_id: Option<Uuid>,
            is_todo: bool,
            todo_due: Option<DateTime<Utc>>,
            todo_completed: Option<DateTime<Utc>>,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            deleted_at: Option<DateTime<Utc>>,
        }
        let ts = now();
        let old = OldNote {
            id: new_id(),
            title: "t",
            body: "b",
            notebook_id: None,
            is_todo: false,
            todo_due: None,
            todo_completed: None,
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
        };
        let bytes = rmp_serde::to_vec_named(&old).unwrap();
        let note: Note = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(note.notebook_id, Uuid::nil());
        assert_eq!(note.sort_key, 0);
    }
}
