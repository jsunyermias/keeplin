use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::StorageError,
    models::{Change, Note, NoteTag, Notebook, Resource, Tag},
    storage::StorageBackend,
};

const NONCE_LEN: usize = 12;

pub struct EncryptedBackend<B: StorageBackend> {
    inner: B,
    cipher: Aes256Gcm,
}

impl<B: StorageBackend> EncryptedBackend<B> {
    /// Create an EncryptedBackend.  The backend's `get_device_id()` is used as
    /// the Argon2id salt so that the derived key is stable per installation but
    /// unique across installations even when the same password is used.
    pub async fn new(inner: B, password: &str) -> Result<Self, StorageError> {
        let device_id = inner.get_device_id().await?;
        let key = derive_key(password, device_id.as_bytes())?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        Ok(Self { inner, cipher })
    }

    fn encrypt_str(&self, plaintext: &str) -> Result<String, StorageError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| StorageError::InvalidState(format!("encrypt: {e}")))?;
        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ct);
        Ok(STANDARD.encode(&combined))
    }

    fn decrypt_str(&self, encoded: &str) -> Result<String, StorageError> {
        let combined = STANDARD
            .decode(encoded)
            .map_err(|e| StorageError::InvalidState(format!("base64: {e}")))?;
        if combined.len() < NONCE_LEN {
            return Err(StorageError::InvalidState("ciphertext too short".into()));
        }
        let nonce = Nonce::from_slice(&combined[..NONCE_LEN]);
        let plain = self
            .cipher
            .decrypt(nonce, &combined[NONCE_LEN..])
            .map_err(|e| StorageError::InvalidState(format!("decrypt: {e}")))?;
        String::from_utf8(plain).map_err(|e| StorageError::InvalidState(format!("utf8: {e}")))
    }

    fn encrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, data)
            .map_err(|e| StorageError::InvalidState(format!("encrypt: {e}")))?;
        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ct);
        Ok(combined)
    }

    fn decrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError> {
        if data.len() < NONCE_LEN {
            return Err(StorageError::InvalidState("ciphertext too short".into()));
        }
        let nonce = Nonce::from_slice(&data[..NONCE_LEN]);
        self.cipher
            .decrypt(nonce, &data[NONCE_LEN..])
            .map_err(|e| StorageError::InvalidState(format!("decrypt: {e}")))
    }

    fn enc_note(&self, mut n: Note) -> Result<Note, StorageError> {
        n.title = self.encrypt_str(&n.title)?;
        n.body = self.encrypt_str(&n.body)?;
        Ok(n)
    }

    fn dec_note(&self, mut n: Note) -> Result<Note, StorageError> {
        n.title = self.decrypt_str(&n.title)?;
        n.body = self.decrypt_str(&n.body)?;
        Ok(n)
    }

    fn enc_notebook(&self, mut nb: Notebook) -> Result<Notebook, StorageError> {
        nb.title = self.encrypt_str(&nb.title)?;
        Ok(nb)
    }

    fn dec_notebook(&self, mut nb: Notebook) -> Result<Notebook, StorageError> {
        nb.title = self.decrypt_str(&nb.title)?;
        Ok(nb)
    }

    fn enc_tag(&self, mut t: Tag) -> Result<Tag, StorageError> {
        t.title = self.encrypt_str(&t.title)?;
        Ok(t)
    }

    fn dec_tag(&self, mut t: Tag) -> Result<Tag, StorageError> {
        t.title = self.decrypt_str(&t.title)?;
        Ok(t)
    }

    fn enc_resource(&self, mut r: Resource) -> Result<Resource, StorageError> {
        r.title = self.encrypt_str(&r.title)?;
        r.mime_type = self.encrypt_str(&r.mime_type)?;
        r.file_name = self.encrypt_str(&r.file_name)?;
        Ok(r)
    }

    fn dec_resource(&self, mut r: Resource) -> Result<Resource, StorageError> {
        r.title = self.decrypt_str(&r.title)?;
        r.mime_type = self.decrypt_str(&r.mime_type)?;
        r.file_name = self.decrypt_str(&r.file_name)?;
        Ok(r)
    }
}

/// Derive a 32-byte AES key using Argon2id.
/// `salt` should be a stable, per-installation value (e.g. the device ID).
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], StorageError> {
    let params = Params::new(65536, 3, 1, Some(32))
        .map_err(|e| StorageError::InvalidState(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| StorageError::InvalidState(format!("kdf: {e}")))?;
    Ok(key)
}

#[async_trait]
impl<B: StorageBackend> StorageBackend for EncryptedBackend<B> {
    async fn create_note(&self, note: Note) -> Result<Note, StorageError> {
        let stored = self.inner.create_note(self.enc_note(note)?).await?;
        self.dec_note(stored)
    }

    async fn read_note(&self, id: Uuid) -> Result<Note, StorageError> {
        self.dec_note(self.inner.read_note(id).await?)
    }

    async fn update_note(&self, note: Note) -> Result<Note, StorageError> {
        let stored = self.inner.update_note(self.enc_note(note)?).await?;
        self.dec_note(stored)
    }

    async fn delete_note(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_note(id).await
    }

    async fn list_notes(&self) -> Result<Vec<Note>, StorageError> {
        self.inner
            .list_notes()
            .await?
            .into_iter()
            .map(|n| self.dec_note(n))
            .collect()
    }

    async fn create_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let stored = self
            .inner
            .create_notebook(self.enc_notebook(notebook)?)
            .await?;
        self.dec_notebook(stored)
    }

    async fn read_notebook(&self, id: Uuid) -> Result<Notebook, StorageError> {
        self.dec_notebook(self.inner.read_notebook(id).await?)
    }

    async fn update_notebook(&self, notebook: Notebook) -> Result<Notebook, StorageError> {
        let stored = self
            .inner
            .update_notebook(self.enc_notebook(notebook)?)
            .await?;
        self.dec_notebook(stored)
    }

    async fn delete_notebook(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_notebook(id).await
    }

    async fn list_notebooks(&self) -> Result<Vec<Notebook>, StorageError> {
        self.inner
            .list_notebooks()
            .await?
            .into_iter()
            .map(|n| self.dec_notebook(n))
            .collect()
    }

    async fn create_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let stored = self.inner.create_tag(self.enc_tag(tag)?).await?;
        self.dec_tag(stored)
    }

    async fn read_tag(&self, id: Uuid) -> Result<Tag, StorageError> {
        self.dec_tag(self.inner.read_tag(id).await?)
    }

    async fn update_tag(&self, tag: Tag) -> Result<Tag, StorageError> {
        let stored = self.inner.update_tag(self.enc_tag(tag)?).await?;
        self.dec_tag(stored)
    }

    async fn delete_tag(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_tag(id).await
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, StorageError> {
        self.inner
            .list_tags()
            .await?
            .into_iter()
            .map(|t| self.dec_tag(t))
            .collect()
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        self.inner.add_note_tag(note_tag).await
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.inner.remove_note_tag(note_id, tag_id).await
    }

    async fn list_note_tags(&self, note_id: Uuid) -> Result<Vec<Tag>, StorageError> {
        self.inner
            .list_note_tags(note_id)
            .await?
            .into_iter()
            .map(|t| self.dec_tag(t))
            .collect()
    }

    async fn create_resource(
        &self,
        resource: Resource,
        data: Vec<u8>,
    ) -> Result<Resource, StorageError> {
        let enc_data = self.encrypt_bytes(&data)?;
        let stored = self
            .inner
            .create_resource(self.enc_resource(resource)?, enc_data)
            .await?;
        self.dec_resource(stored)
    }

    async fn read_resource(&self, id: Uuid) -> Result<(Resource, Vec<u8>), StorageError> {
        let (res, enc_data) = self.inner.read_resource(id).await?;
        let data = self.decrypt_bytes(&enc_data)?;
        Ok((self.dec_resource(res)?, data))
    }

    async fn delete_resource(&self, id: Uuid) -> Result<(), StorageError> {
        self.inner.delete_resource(id).await
    }

    async fn list_resources(&self) -> Result<Vec<Resource>, StorageError> {
        self.inner
            .list_resources()
            .await?
            .into_iter()
            .map(|r| self.dec_resource(r))
            .collect()
    }

    // Sync methods pass through: encrypted data travels and is stored encrypted.
    async fn get_changes_since(&self, since: DateTime<Utc>) -> Result<Vec<Change>, StorageError> {
        self.inner.get_changes_since(since).await
    }

    async fn apply_change(&self, change: Change) -> Result<(), StorageError> {
        self.inner.apply_change(change).await
    }

    async fn get_last_sync_time(&self) -> Result<DateTime<Utc>, StorageError> {
        self.inner.get_last_sync_time().await
    }

    async fn update_sync_time(&self, ts: DateTime<Utc>) -> Result<(), StorageError> {
        self.inner.update_sync_time(ts).await
    }

    async fn send_changes(&self, changes: Vec<Change>) -> Result<(), StorageError> {
        self.inner.send_changes(changes).await
    }

    async fn receive_changes(&self) -> Result<Vec<Change>, StorageError> {
        self.inner.receive_changes().await
    }

    async fn get_device_id(&self) -> Result<String, StorageError> {
        self.inner.get_device_id().await
    }
}
