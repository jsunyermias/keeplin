use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn new_id() -> Uuid {
    Uuid::new_v4()
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub notebook_id: Option<Uuid>,
    pub is_todo: bool,
    pub todo_due: Option<DateTime<Utc>>,
    pub todo_completed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Note {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            body: body.into(),
            notebook_id: None,
            is_todo: false,
            todo_due: None,
            todo_completed: None,
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Notebook {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Notebook {
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Tag {
    pub fn new(title: impl Into<String>) -> Self {
        let ts = now();
        Self {
            id: new_id(),
            title: title.into(),
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteTag {
    pub note_id: Uuid,
    pub tag_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Resource {
    pub id: Uuid,
    pub title: String,
    pub mime_type: String,
    pub file_name: String,
    pub size: u64,
    pub created_at: DateTime<Utc>,
}

impl Resource {
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    Create { note: Note },
    Update { note: Note },
    Delete { id: Uuid },
}
