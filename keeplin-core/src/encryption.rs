//! Transparent at-rest encryption decorator for any [`StorageBackend`].
//!
//! [`EncryptedBackend<B>`] wraps any `B: StorageBackend` and automatically encrypts
//! sensitive fields before they are written to the inner backend, then decrypts them
//! on the way back out. Callers interact with the encrypted backend through the same
//! `StorageBackend` trait as a plain backend — encryption is completely transparent.
//!
//! # Encryption scheme
//!
//! - Cipher: **AES-256-GCM** (authenticated encryption; any tampering is detected).
//! - Key derivation: **Argon2id** (memory = 64 MiB, iterations = 3, parallelism = 1).
//! - Salt: the device ID returned by the inner backend. Using the device ID as the salt
//!   ensures that the same passphrase produces a different key on every installation,
//!   preventing cross-device key reuse.
//! - Nonce: 12 random bytes generated fresh for **every** encryption call.
//! - Wire format (strings): `base64(nonce ‖ ciphertext)`.
//! - Wire format (bytes): raw `nonce ‖ ciphertext` bytes (no Base64 for binary data).

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
    storage::{
        NoteRepository, NotebookRepository, ResourceRepository, StorageBackend, SyncBackend,
        TagRepository,
    },
};

/// Length in bytes of the AES-GCM nonce. AES-GCM is specified with a 96-bit (12-byte)
/// nonce; this value must not be changed without also changing the cipher.
const NONCE_LEN: usize = 12;

/// Transparent AES-256-GCM encryption wrapper around any [`StorageBackend`].
///
/// Sensitive string fields (`Note.title`, `Note.body`, `Notebook.title`, `Tag.title`,
/// `Resource.title`, `Resource.mime_type`, `Resource.file_name`) and binary resource
/// payloads are encrypted before being passed to the inner backend. All other fields
/// (UUIDs, timestamps, sizes, association tables) are stored in plaintext because they
/// are needed for queries and contain no user-supplied content that requires protection.
pub struct EncryptedBackend<B: StorageBackend> {
    /// The underlying backend that stores (encrypted) data. All read/write operations
    /// ultimately go through this field.
    inner: B,
    /// The AES-256-GCM cipher instance, initialised once with the Argon2id-derived key.
    cipher: Aes256Gcm,
}

impl<B: StorageBackend> EncryptedBackend<B> {
    /// Constructs an `EncryptedBackend` wrapping `inner`.
    ///
    /// Calls `inner.get_device_id()` to obtain the Argon2id salt, then derives a
    /// 256-bit AES key from `password` and that salt. The key is stable per
    /// installation (the device ID is persisted on disk) but unique across installations
    /// even when the same password is used.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::InvalidState` if Argon2id parameter construction fails or
    /// the inner backend's `get_device_id()` returns an error.
    pub async fn new(inner: B, password: &str) -> Result<Self, StorageError> {
        let device_id = inner.get_device_id().await?;
        let key = derive_key(password, device_id.as_bytes())?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        Ok(Self { inner, cipher })
    }

    /// Encrypts a plaintext string and returns `base64(nonce ‖ ciphertext)`.
    ///
    /// A fresh 12-byte random nonce is generated for every call so that the same
    /// plaintext encrypted twice produces two different ciphertexts. This is required
    /// for semantic security under AES-GCM.
    fn encrypt_str(&self, plaintext: &str) -> Result<String, StorageError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| StorageError::InvalidState(format!("encrypt: {e}")))?;
        // Prepend the nonce to the ciphertext so the decrypt function can extract it
        // without storing it separately. Then Base64-encode the combined buffer so the
        // result is a plain ASCII string that can be stored in a JSON field.
        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ct);
        Ok(STANDARD.encode(&combined))
    }

    /// Decrypts a string previously produced by [`encrypt_str`].
    ///
    /// Decodes Base64, extracts the 12-byte nonce from the front of the buffer,
    /// decrypts the remaining bytes with AES-GCM, and interprets the result as UTF-8.
    ///
    /// Returns `StorageError::InvalidState` if the Base64, the AES-GCM authentication
    /// tag, or the UTF-8 conversion fails. A wrong decryption key causes the AES-GCM
    /// authentication tag to fail, which surfaces here as `InvalidState`.
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
            .map_err(|e| StorageError::CorruptedData(format!("decrypt: {e}")))?;
        String::from_utf8(plain).map_err(|e| StorageError::InvalidState(format!("utf8: {e}")))
    }

    /// Encrypts raw bytes and returns `nonce ‖ ciphertext` as a byte vector.
    ///
    /// Unlike `encrypt_str`, the result is not Base64-encoded because the caller
    /// stores the bytes directly in a binary column or file.
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

    /// Decrypts bytes previously produced by [`encrypt_bytes`].
    ///
    /// Extracts the 12-byte nonce from the front of the slice and decrypts the rest.
    fn decrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>, StorageError> {
        if data.len() < NONCE_LEN {
            return Err(StorageError::InvalidState("ciphertext too short".into()));
        }
        let nonce = Nonce::from_slice(&data[..NONCE_LEN]);
        self.cipher
            .decrypt(nonce, &data[NONCE_LEN..])
            .map_err(|e| StorageError::CorruptedData(format!("decrypt: {e}")))
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

/// Derives a 32-byte (256-bit) AES key from a password and a salt using Argon2id.
///
/// Parameters chosen for a balance between security and performance on typical
/// desktop hardware (approximately 300 ms on a modern laptop):
/// - Memory: 64 MiB (`65536` KiB)
/// - Iterations: 3
/// - Parallelism: 1 (single-threaded)
/// - Output length: 32 bytes
///
/// `salt` must be a stable, per-installation byte sequence (e.g. the device ID string)
/// so that the derived key is different on every device even when the same password is
/// used. The salt does not need to be secret, but it must be persisted across restarts.
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
impl<B: StorageBackend> NoteRepository for EncryptedBackend<B> {
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

    async fn list_notes(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Note>, Option<String>), StorageError> {
        let (notes, next) = self.inner.list_notes(page_size, page_token).await?;
        let decrypted: Result<Vec<Note>, StorageError> =
            notes.into_iter().map(|n| self.dec_note(n)).collect();
        Ok((decrypted?, next))
    }
}

#[async_trait]
impl<B: StorageBackend> NotebookRepository for EncryptedBackend<B> {
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

    async fn list_notebooks(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Notebook>, Option<String>), StorageError> {
        let (notebooks, next) = self.inner.list_notebooks(page_size, page_token).await?;
        let decrypted: Result<Vec<Notebook>, StorageError> = notebooks
            .into_iter()
            .map(|nb| self.dec_notebook(nb))
            .collect();
        Ok((decrypted?, next))
    }
}

#[async_trait]
impl<B: StorageBackend> TagRepository for EncryptedBackend<B> {
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

    async fn list_tags(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let (tags, next) = self.inner.list_tags(page_size, page_token).await?;
        let decrypted: Result<Vec<Tag>, StorageError> =
            tags.into_iter().map(|t| self.dec_tag(t)).collect();
        Ok((decrypted?, next))
    }

    async fn add_note_tag(&self, note_tag: NoteTag) -> Result<(), StorageError> {
        self.inner.add_note_tag(note_tag).await
    }

    async fn remove_note_tag(&self, note_id: Uuid, tag_id: Uuid) -> Result<(), StorageError> {
        self.inner.remove_note_tag(note_id, tag_id).await
    }

    async fn list_note_tags(
        &self,
        note_id: Uuid,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Tag>, Option<String>), StorageError> {
        let (tags, next) = self
            .inner
            .list_note_tags(note_id, page_size, page_token)
            .await?;
        let decrypted: Result<Vec<Tag>, StorageError> =
            tags.into_iter().map(|t| self.dec_tag(t)).collect();
        Ok((decrypted?, next))
    }
}

#[async_trait]
impl<B: StorageBackend> ResourceRepository for EncryptedBackend<B> {
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

    async fn list_resources(
        &self,
        page_size: u32,
        page_token: Option<String>,
    ) -> Result<(Vec<Resource>, Option<String>), StorageError> {
        let (resources, next) = self.inner.list_resources(page_size, page_token).await?;
        let decrypted: Result<Vec<Resource>, StorageError> = resources
            .into_iter()
            .map(|r| self.dec_resource(r))
            .collect();
        Ok((decrypted?, next))
    }
}

// Synchronisation methods pass through without any transformation. The data that
// travels over the sync channel is already in the encrypted form that the inner
// backend stored on disk, so no additional encryption or decryption step is needed.
#[async_trait]
impl<B: StorageBackend> SyncBackend for EncryptedBackend<B> {
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

    async fn prune_change_journal(&self, older_than: DateTime<Utc>) -> Result<u64, StorageError> {
        self.inner.prune_change_journal(older_than).await
    }
}
