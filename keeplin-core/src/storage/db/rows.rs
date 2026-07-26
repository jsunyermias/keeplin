// md:Overview

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, Notebook, Resource, Tag},
};

use super::convert::{
    assoc_from_data, json_to_bookmarks, json_to_links, json_to_vv, tombstone_from_data,
};
use super::DbBackend;

// md:impl DbBackend (row mapping)
impl DbBackend {
    // md:impl DbBackend (row mapping) > fn row_to_note
    pub(super) fn row_to_note(row: &libsql::Row) -> Result<Note, StorageError> {
        let id = Self::parse_uuid(row.get::<String>(0)?)?;
        let title: String = row.get(1)?;
        let body: String = row.get(2)?;
        let notebook_id: Uuid = row
            .get::<Option<String>>(3)?
            .map(Self::parse_uuid)
            .transpose()?
            .unwrap_or_else(Uuid::nil);
        let is_todo: bool = row.get::<i64>(4)? != 0;
        let todo_due = Self::parse_optional_dt(row.get::<Option<String>>(5)?)?;
        let todo_completed = Self::parse_optional_dt(row.get::<Option<String>>(6)?)?;
        let created_at = Self::parse_required_dt(row.get::<String>(7)?)?;
        let updated_at = Self::parse_required_dt(row.get::<String>(8)?)?;
        let deleted_at = Self::parse_optional_dt(row.get::<Option<String>>(9)?)?;
        let alias: Option<String> = row.get(10)?;
        let bookmarks = json_to_bookmarks(&row.get::<String>(11)?);
        let links = json_to_links(&row.get::<String>(12)?);
        let vv = json_to_vv(&row.get::<String>(13)?);
        let last_writer: String = row.get(14)?;
        let is_pinned: bool = row.get::<i64>(15)? != 0;
        let is_starred: bool = row.get::<i64>(16)? != 0;
        let sort_key: u32 = row.get::<i64>(17)?.max(0) as u32;

        Ok(Note {
            id,
            title,
            body,
            notebook_id,
            is_todo,
            todo_due,
            todo_completed,
            created_at,
            updated_at,
            deleted_at,
            alias,
            bookmarks,
            links,
            vv,
            last_writer,
            is_pinned,
            is_starred,
            sort_key,
        })
    }

    // md:impl DbBackend (row mapping) > fn parse_uuid
    fn parse_uuid(s: String) -> Result<Uuid, StorageError> {
        s.parse()
            .map_err(|e: uuid::Error| StorageError::InvalidState(e.to_string()))
    }

    // md:impl DbBackend (row mapping) > fn parse_required_dt
    pub(super) fn parse_required_dt(s: String) -> Result<DateTime<Utc>, StorageError> {
        s.parse::<DateTime<Utc>>()
            .map_err(|e| StorageError::InvalidState(e.to_string()))
    }

    // md:impl DbBackend (row mapping) > fn parse_optional_dt
    pub(super) fn parse_optional_dt(
        s: Option<String>,
    ) -> Result<Option<DateTime<Utc>>, StorageError> {
        match s {
            None => Ok(None),
            Some(v) => v
                .parse::<DateTime<Utc>>()
                .map(Some)
                .map_err(|e| StorageError::InvalidState(e.to_string())),
        }
    }

    // md:impl DbBackend (row mapping) > fn row_to_notebook
    pub(super) fn row_to_notebook(row: &libsql::Row) -> Result<Notebook, StorageError> {
        Ok(Notebook {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            created_at: Self::parse_required_dt(row.get::<String>(2)?)?,
            updated_at: Self::parse_required_dt(row.get::<String>(3)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(4)?)?,
            alias: row.get(5)?,
            vv: json_to_vv(&row.get::<String>(6)?),
            last_writer: row.get(7)?,
        })
    }

    // md:impl DbBackend (row mapping) > fn row_to_tag
    pub(super) fn row_to_tag(row: &libsql::Row) -> Result<Tag, StorageError> {
        Ok(Tag {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            title: row.get(1)?,
            created_at: Self::parse_required_dt(row.get::<String>(2)?)?,
            updated_at: Self::parse_required_dt(row.get::<String>(3)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(4)?)?,
            vv: json_to_vv(&row.get::<String>(5)?),
            last_writer: row.get(6)?,
            system: row.get::<i64>(7)? != 0,
        })
    }

    // md:impl DbBackend (row mapping) > fn row_to_resource
    pub(super) fn row_to_resource(row: &libsql::Row) -> Result<Resource, StorageError> {
        let width = row.get::<Option<i64>>(10)?;
        let height = row.get::<Option<i64>>(11)?;
        let dimensions = match (width, height) {
            (Some(w), Some(h)) => Some((w as u32, h as u32)),
            _ => None,
        };
        Ok(Resource {
            id: Self::parse_uuid(row.get::<String>(0)?)?,
            note_id: Self::parse_uuid(row.get::<String>(12)?)?,
            title: row.get(1)?,
            mime_type: row.get(2)?,
            file_name: row.get(3)?,
            size: row.get::<i64>(4)? as u64,
            duration_ms: row.get::<Option<i64>>(9)?.map(|v| v as u64),
            dimensions,
            created_at: Self::parse_required_dt(row.get::<String>(5)?)?,
            deleted_at: Self::parse_optional_dt(row.get::<Option<String>>(6)?)?,
            vv: json_to_vv(&row.get::<String>(7)?),
            last_writer: row.get(8)?,
        })
    }

    // md:impl DbBackend (row mapping) > fn row_to_change
    pub(super) fn row_to_change(
        entity_type: &str,
        entity_id_str: &str,
        operation: &str,
        changed_at: DateTime<Utc>,
        data: &serde_json::Value,
    ) -> Option<Change> {
        let id: Uuid = entity_id_str.parse().ok()?;
        match (entity_type, operation) {
            ("note", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|note| Change::NoteCreate { note }),
            ("note", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|note| Change::NoteUpdate { note }),
            ("note", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
                Some(Change::NoteDelete {
                    id,
                    deleted_at,
                    vv,
                    last_writer,
                })
            }
            ("notebook", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|notebook| Change::NotebookCreate { notebook }),
            ("notebook", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|notebook| Change::NotebookUpdate { notebook }),
            ("notebook", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
                Some(Change::NotebookDelete {
                    id,
                    deleted_at,
                    vv,
                    last_writer,
                })
            }
            ("tag", "create") => serde_json::from_value(data.clone())
                .ok()
                .map(|tag| Change::TagCreate { tag }),
            ("tag", "update") => serde_json::from_value(data.clone())
                .ok()
                .map(|tag| Change::TagUpdate { tag }),
            ("tag", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
                Some(Change::TagDelete {
                    id,
                    deleted_at,
                    vv,
                    last_writer,
                })
            }
            ("note_tag", "add") => {
                let tag_id: Uuid = data["tag_id"].as_str()?.parse().ok()?;
                let (updated_at, vv, last_writer) = assoc_from_data(data, changed_at);
                Some(Change::NoteTagAdd {
                    note_id: id,
                    tag_id,
                    updated_at,
                    vv,
                    last_writer,
                })
            }
            ("note_tag", "remove") => {
                let tag_id: Uuid = data["tag_id"].as_str()?.parse().ok()?;
                let (updated_at, vv, last_writer) = assoc_from_data(data, changed_at);
                Some(Change::NoteTagRemove {
                    note_id: id,
                    tag_id,
                    updated_at,
                    vv,
                    last_writer,
                })
            }
            ("resource", "create") => {
                let binary = data["_data_b64"]
                    .as_str()
                    .and_then(|b| STANDARD.decode(b).ok());
                serde_json::from_value(data.clone())
                    .ok()
                    .map(|resource| Change::ResourceCreate {
                        resource,
                        data: binary,
                    })
            }
            ("resource", "delete") => {
                let (deleted_at, vv, last_writer) = tombstone_from_data(data, changed_at);
                Some(Change::ResourceDelete {
                    id,
                    deleted_at,
                    vv,
                    last_writer,
                })
            }
            _ => None,
        }
    }
}
