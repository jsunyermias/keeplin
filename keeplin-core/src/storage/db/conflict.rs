// md:Overview

use chrono::{DateTime, Utc};

use crate::error::StorageError;

use crate::storage::note_log::{self, resolve, VersionVector, Winner};
use crate::storage::SortableRfc3339;

use super::convert::{json_to_vv, vv_to_json};
use super::DbBackend;

// md:impl DbBackend (conflict resolution)
impl DbBackend {
    // md:impl DbBackend (conflict resolution) > fn current_meta
    async fn current_meta(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<(VersionVector, DateTime<Utc>, String)>, StorageError> {
        let mut rows = self
            .conn
            .query(
                &format!("SELECT vv, updated_at, last_writer FROM {table} WHERE id = ?1"),
                [id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(Some((
                json_to_vv(&row.get::<String>(0)?),
                Self::parse_required_dt(row.get::<String>(1)?)?,
                row.get::<String>(2)?,
            ))),
            None => Ok(None),
        }
    }

    // md:impl DbBackend (conflict resolution) > fn incoming_wins
    pub(super) async fn incoming_wins(
        &self,
        table: &str,
        id: &str,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.current_meta(table, id).await? {
            None => Ok(true),
            Some((local_vv, local_updated, local_writer)) => Ok(matches!(
                resolve(
                    &local_vv,
                    local_updated,
                    &local_writer,
                    incoming_vv,
                    incoming_updated,
                    incoming_writer,
                ),
                Winner::Incoming
            )),
        }
    }

    // md:impl DbBackend (conflict resolution) > fn next_local_vv
    pub(super) async fn next_local_vv(
        &self,
        table: &str,
        id: &str,
    ) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .current_meta(table, id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl DbBackend (conflict resolution) > fn row_is_live
    pub(super) async fn row_is_live(&self, table: &str, id: &str) -> Result<bool, StorageError> {
        let mut rows = self
            .conn
            .query(
                &format!("SELECT deleted_at FROM {table} WHERE id = ?1"),
                [id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<Option<String>>(0)?.is_none()),
            None => Ok(false),
        }
    }

    // md:impl DbBackend (conflict resolution) > fn assoc_meta
    async fn assoc_meta(
        &self,
        note_id: &str,
        tag_id: &str,
    ) -> Result<Option<(VersionVector, DateTime<Utc>, String)>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT vv, updated_at, last_writer FROM note_tags WHERE note_id=?1 AND tag_id=?2",
                [note_id.to_owned(), tag_id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let updated_at = match row.get::<Option<String>>(1)? {
                    Some(s) => Self::parse_required_dt(s)?,
                    None => DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default(),
                };
                Ok(Some((
                    json_to_vv(&row.get::<String>(0)?),
                    updated_at,
                    row.get::<String>(2)?,
                )))
            }
            None => Ok(None),
        }
    }

    // md:impl DbBackend (conflict resolution) > fn next_assoc_vv
    pub(super) async fn next_assoc_vv(
        &self,
        note_id: &str,
        tag_id: &str,
    ) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .assoc_meta(note_id, tag_id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl DbBackend (conflict resolution) > fn assoc_incoming_wins
    pub(super) async fn assoc_incoming_wins(
        &self,
        note_id: &str,
        tag_id: &str,
        incoming_vv: &VersionVector,
        incoming_updated: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.assoc_meta(note_id, tag_id).await? {
            None => Ok(true),
            Some((lvv, lupd, lwriter)) => Ok(matches!(
                resolve(
                    &lvv,
                    lupd,
                    &lwriter,
                    incoming_vv,
                    incoming_updated,
                    incoming_writer
                ),
                Winner::Incoming
            )),
        }
    }

    // md:impl DbBackend (conflict resolution) > fn upsert_assoc
    pub(super) async fn upsert_assoc(
        &self,
        note_id: &str,
        tag_id: &str,
        updated_at: DateTime<Utc>,
        deleted_at: Option<DateTime<Utc>>,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO note_tags (note_id,tag_id,updated_at,deleted_at,vv,last_writer)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                libsql::params![
                    note_id.to_owned(),
                    tag_id.to_owned(),
                    updated_at.to_sortable_rfc3339(),
                    deleted_at.map(|d| d.to_sortable_rfc3339()),
                    vv_to_json(vv),
                    last_writer.to_owned(),
                ],
            )
            .await?;
        Ok(())
    }

    // md:impl DbBackend (conflict resolution) > fn resource_meta
    async fn resource_meta(
        &self,
        id: &str,
    ) -> Result<Option<(VersionVector, DateTime<Utc>, String)>, StorageError> {
        let mut rows = self
            .conn
            .query(
                "SELECT vv, created_at, deleted_at, last_writer FROM resources WHERE id=?1",
                [id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let created_at = Self::parse_required_dt(row.get::<String>(1)?)?;
                let deleted_at = Self::parse_optional_dt(row.get::<Option<String>>(2)?)?;
                Ok(Some((
                    json_to_vv(&row.get::<String>(0)?),
                    deleted_at.unwrap_or(created_at),
                    row.get::<String>(3)?,
                )))
            }
            None => Ok(None),
        }
    }

    // md:impl DbBackend (conflict resolution) > fn next_resource_vv
    pub(super) async fn next_resource_vv(&self, id: &str) -> Result<VersionVector, StorageError> {
        let mut vv = self
            .resource_meta(id)
            .await?
            .map(|(vv, _, _)| vv)
            .unwrap_or_default();
        note_log::increment(&mut vv, &self.device_id);
        Ok(vv)
    }

    // md:impl DbBackend (conflict resolution) > fn resource_incoming_wins
    pub(super) async fn resource_incoming_wins(
        &self,
        id: &str,
        incoming_vv: &VersionVector,
        incoming_ts: DateTime<Utc>,
        incoming_writer: &str,
    ) -> Result<bool, StorageError> {
        match self.resource_meta(id).await? {
            None => Ok(true),
            Some((lvv, lts, lwriter)) => Ok(matches!(
                resolve(
                    &lvv,
                    lts,
                    &lwriter,
                    incoming_vv,
                    incoming_ts,
                    incoming_writer
                ),
                Winner::Incoming
            )),
        }
    }
}
