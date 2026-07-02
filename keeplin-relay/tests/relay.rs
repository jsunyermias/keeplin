//! End-to-end tests for `keeplin-relay`: stand the real relay up on an ephemeral port and drive
//! genuine `DbBackend` instances through it over a socket, plus an auth-rejection check.

use std::time::Duration;

use keeplin_core::{
    models::Note,
    storage::{db::DbBackend, NoteRepository, SyncBackend},
};
use keeplin_relay::{serve, RelayConfig};
use tokio::net::TcpListener;
use uuid::Uuid;

/// Start the relay on an ephemeral port with the given token and return its `ws://` URL. The
/// relay task runs until the test process exits.
async fn spawn_relay(token: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = token.to_string();
    tokio::spawn(async move {
        serve(
            listener,
            RelayConfig { auth_token: token },
            std::future::pending::<()>(), // never shut down during the test
        )
        .await
        .unwrap();
    });
    format!("ws://{addr}")
}

/// A server-mode `DbBackend` connected to `url` with `token`. The temp dir is leaked so it
/// outlives the open database for the test.
async fn device(url: &str, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, url, token).await.unwrap()
}

fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap()
}

/// Poll `receive_changes` + `apply_change` for up to ~3 s until note `id` is present on `dev`.
async fn sync_until_present(dev: &DbBackend, id: Uuid) -> bool {
    for _ in 0..30 {
        for change in dev.receive_changes().await.unwrap() {
            dev.apply_change(change).await.unwrap();
        }
        if dev.read_note(id).await.is_ok() {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn note_created_on_one_device_reaches_another_through_the_relay() {
    let url = spawn_relay("s3cr3t").await;
    let a = device(&url, "s3cr3t").await;
    let b = device(&url, "s3cr3t").await;

    // Device A creates a note and pushes its changes to the relay.
    let note = Note::new("Shared", "over the real relay");
    let id = note.id;
    a.create_note(note).await.unwrap();
    let changes = a.get_changes_since(epoch()).await.unwrap();
    a.send_changes(changes).await.unwrap();

    // The relay forwards them to device B, which converges on the note.
    assert!(
        sync_until_present(&b, id).await,
        "device B must receive A's note through the relay"
    );
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Shared");
    assert_eq!(read.body, "over the real relay");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_with_the_wrong_token_cannot_sync() {
    let url = spawn_relay("correct-token").await;

    // The sender authenticates correctly; the receiver presents the wrong token, so the relay
    // closes its connection and it never receives the broadcast.
    let a = device(&url, "correct-token").await;
    let b = device(&url, "wrong-token").await;

    let note = Note::new("secret", "should not arrive");
    let id = note.id;
    a.create_note(note).await.unwrap();
    let changes = a.get_changes_since(epoch()).await.unwrap();
    a.send_changes(changes).await.unwrap();

    // Give the (rejected) receiver time to attempt draining, then assert it saw nothing.
    for _ in 0..5 {
        for change in b.receive_changes().await.unwrap() {
            b.apply_change(change).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        b.read_note(id).await.is_err(),
        "a device that failed auth must not receive changes"
    );
}
