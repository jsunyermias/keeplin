# `tests/encryption.rs` — EncryptedBackend integration tests

## What is tested

This file contains integration tests for `EncryptedBackend<FsBackend>`. Each test creates
a temporary directory, wraps a fresh `FsBackend` in an `EncryptedBackend`, and verifies
that the encryption layer behaves correctly: plaintext is never stored on disk in readable
form, round-trips produce the original plaintext, and wrong passwords are correctly
rejected.

## Fixtures and helpers

| Helper | Purpose |
|--------|---------|
| `async fn enc_backend(dir: &Path) -> EncryptedBackend<FsBackend>` | Creates an `FsBackend` in `dir` and wraps it with an `EncryptedBackend` using the test password `"test-password"` |

## Test cases

| Test function | Scenario | Expected outcome |
|---------------|----------|-----------------|
| `note_round_trips` | Create a note through the encrypted backend, read it back | `title` and `body` match the original plaintext values |
| `storage_contains_ciphertext_not_plaintext` | Create a note, then read the raw `meta.json` file from disk | The file does not contain the plaintext title or body strings |
| `wrong_password_fails_to_decrypt` | Write a note with the correct password, then open the same directory with a wrong password and attempt to read | `read_note` returns an error (AES-GCM authentication failure) |
| `update_note_encrypts_new_content` | Create a note, update both `title` and `body`, read back | New plaintext values are returned correctly (new ciphertext stored) |
| `list_notes_decrypts_all` | Create three notes, call `list_notes` | Returns all three notes with decrypted titles |
| `notebook_round_trips` | Create a notebook, read it back | `title` matches |
| `tag_round_trips` | Create a tag, read it back | `title` matches |
| `note_tag_relation_preserved` | Create a note and a tag through the encrypted backend, link them, list tags for the note | Returns one tag with the expected (decrypted) title |
| `resource_round_trips` | Create a resource with binary data, read it back | Metadata fields match; binary bytes are identical to the originals |
| `resource_data_stored_encrypted` | Create a resource, read the raw `data` file from disk | Raw bytes do not equal the plaintext payload `b"supersecret"` |

## Design notes on the tests

- `storage_contains_ciphertext_not_plaintext` and `resource_data_stored_encrypted` bypass
  the backend API and read the raw filesystem directly to confirm that the encryption
  layer is actually active. Without these checks, a bug that skips encryption would still
  pass all round-trip tests.
- `wrong_password_fails_to_decrypt` creates two separate `FsBackend` instances pointing to
  the same directory. This is safe in the test context because the two instances are never
  used concurrently.

## Coverage gaps

- `EncryptedBackend` wrapping `DbBackend` is not tested (only `FsBackend` is exercised
  here). The encryption logic is identical for both because it operates entirely on the
  domain types before they reach the inner backend.
- Sync methods (`get_changes_since`, `apply_change`, `send_changes`, `receive_changes`)
  are not tested here because `EncryptedBackend` passes them through unchanged.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `enc_backend()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `note_round_trips()` — defined here (EXTRACTED; file-local)
- `storage_contains_ciphertext_not_plaintext()` — defined here (EXTRACTED; file-local)
- `wrong_password_fails_to_decrypt()` — defined here (EXTRACTED; file-local)
- `update_note_encrypts_new_content()` — defined here (EXTRACTED; file-local)
- `list_notes_decrypts_all()` — defined here (EXTRACTED; file-local)
- `notebook_round_trips()` — defined here (EXTRACTED; file-local)
- `tag_round_trips()` — defined here (EXTRACTED; file-local)
- `note_tag_relation_preserved()` — defined here (EXTRACTED; file-local)
- `resource_round_trips()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/encryption.rs` — transparent at-rest encryption (EXTRACTED: references×1; e.g. `EncryptedBackend`)
- `keeplin-core/src/storage/fs.rs` — FsBackend (filesystem storage) (EXTRACTED: references×1; e.g. `FsBackend`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Plaintext must never be readable in the underlying store; wrong passwords must fail loudly, not return garbage.
- Round-trips return the original plaintext byte-for-byte.

## Related files

- `keeplin-core/src/encryption.rs` — the code under test
- `keeplin-core/tests/fs_backend.rs` — tests the same `FsBackend` without encryption
