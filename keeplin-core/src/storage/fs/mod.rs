// md:Overview
mod convert;
mod history;
mod io;
mod journal;
mod lifecycle;
mod notebooks;
mod notes;
mod pagination;
mod resources;
mod sidecars;
mod sync;
mod tags;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use notes::NoteMetaIndex;

// md:FsBackend
pub struct FsBackend {
    root: PathBuf,
    device_id: String,
    note_write_lock: Arc<Mutex<()>>,
    global_log_lock: Arc<Mutex<()>>,
    note_index: Arc<RwLock<Option<NoteMetaIndex>>>,
}
