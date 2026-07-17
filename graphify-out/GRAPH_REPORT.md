# Graph Report - keeplin  (2026-07-17)

## Corpus Check
- 101 files · ~156,365 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2289 nodes · 6801 edges · 100 communities (95 shown, 5 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 24 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `073786fa`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- FsBackend
- rest.rs
- StorageError
- DbBackend
- CollabBackend<B>
- StorageBackend
- main.rs
- MetricsBackend<B>
- ws_sync.rs
- fs_backend.rs
- `{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}
- search.rs
- KeeplinServer<B>
- in_memory_backend
- Result
- server.rs
- note_log.rs
- LinkingBackend<B>
- linking.rs
- links.rs
- Change
- `storage/db.rs` — DbBackend (LibSQL + WebSocket storage)
- Message types
- Vec
- history.rs
- migrate
- gRPC methods
- Response
- `models.rs` — domain data types
- .update_notebook
- Status
- Note
- `storage/fs.rs` — FsBackend (filesystem storage)
- `src/main.rs` — daemon entry point
- `encryption.rs` — transparent at-rest encryption
- run_sync
- `ordering.rs` — the Inbox, pinning, manual ordering, and starring
- Test cases
- enc_backend
- Test cases
- `src/config.rs` — daemon configuration
- Option
- `collab/mod.rs` — client of the keeplin-srv collaborative channel
- Public API
- Keeplin
- Keeplin — Architecture overview
- `linking.rs` — `LinkingBackend` decorator + reference resolution
- `rest.rs` — REST/JSON API + WebSocket feed (axum)
- compat.rs
- `links.rs` — bookmark & link types and pure parsing
- `migrate.rs` — one-shot state copy between backends
- `storage/backend.rs` — the `StorageBackend` supertrait
- device
- `Cargo.toml` — workspace root
- `.github/workflows/ci.yml` — CI pipeline
- `collab/state.rs` — client line state and body↔lines translation
- `history.rs` — change history reads + forward-revert
- `sync/engine.rs` — SyncEngine
- auth.rs
- `scripts/build.sh` — cross-compilation script
- `error.rs` — error types
- `interop.rs` — vCard & iCalendar format compatibility
- mod.rs
- `storage/mod.rs` — storage module root
- `auth.rs` — shared HTTP Basic authentication
- `event_backend.rs` — `EventBackend` change-publishing decorator
- `search.rs` — daemon full-text search
- Security
- `.cargo/config.toml` — workspace Cargo configuration
- `Cargo.toml` — keeplin-core
- `collab/protocol.rs` — collaborative channel wire types
- `compat.rs` — keeplin-srv protocol/capability handshake (`GET /version`)
- `tests/encryption.rs` — EncryptedBackend integration tests
- `Cargo.toml` — keeplin-daemon
- `src/proto.rs` — generated Protocol Buffers code
- [Unreleased]
- finding.md
- `lib.rs` — keeplin-core crate root
- `sync/mod.rs` — sync module root
- `tests/migrate.rs` — cross-backend migration tests
- `build.rs` — keeplin-daemon build script
- `keeplin-daemon/src/metrics.rs` — operational metrics
- Quick start
- `tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e)
- `tests/sync.rs` — cross-device change-propagation tests
- `tests/version_handshake.rs` — startup protocol handshake tests
- `tests/ws_sync.rs` — end-to-end WebSocket sync test
- main
- Design decisions
- Architecture
- Encryption
- .delete_tag
- .get_tag
- build.sh
- CLAUDE.md
- check-docs.sh

## God Nodes (most connected - your core abstractions)
1. `StorageError` - 406 edges
2. `Note` - 151 edges
3. `FsBackend` - 124 edges
4. `Shared` - 110 edges
5. `DbBackend` - 100 edges
6. `Notebook` - 67 edges
7. `ApiError` - 65 edges
8. `StorageBackend` - 64 edges
9. `EncryptedBackend<B>` - 56 edges
10. `Change` - 56 edges

## Surprising Connections (you probably didn't know these)
- `add_link()` --calls--> `parse_link_ref()`  [INFERRED]
  keeplin-daemon/src/rest.rs → keeplin-core/src/links.rs
- `sync()` --calls--> `run_sync()`  [INFERRED]
  keeplin-daemon/src/rest.rs → keeplin-core/src/sync/engine.rs
- `collab_config()` --references--> `CollabConfig`  [EXTRACTED]
  keeplin-daemon/src/main.rs → keeplin-core/src/collab/mod.rs
- `run_server_with()` --references--> `CollabHandle`  [EXTRACTED]
  keeplin-daemon/src/main.rs → keeplin-core/src/collab/mod.rs
- `AppState` --references--> `CollabHandle`  [EXTRACTED]
  keeplin-daemon/src/rest.rs → keeplin-core/src/collab/mod.rs

## Import Cycles
- None detected.

## Communities (100 total, 5 thin omitted)

### Community 0 - "FsBackend"
Cohesion: 0.06
Nodes (62): BinaryHeap, Eq, T, atomic_write(), compaction_declines_on_unreadable_sidecar_and_resumes_after_repair(), concurrent_same_note_updates_keep_every_log_entry(), corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it(), default_entity_type() (+54 more)

### Community 1 - "rest.rs"
Cohesion: 0.06
Nodes (158): Body, Bytes, HeaderMap, IntoResponse, Json, Client, HashMap, Shared (+150 more)

### Community 2 - "StorageError"
Cohesion: 0.06
Nodes (37): Aes256Gcm, derive_key(), EncryptedBackend, EncryptedBackend<B>, B, DateTime, Option, Result (+29 more)

### Community 3 - "DbBackend"
Cohesion: 0.08
Nodes (45): AtomicBool, Bookmark, assoc_data(), assoc_from_data(), bookmarks_to_json(), build_page(), CapabilityCache, DbBackend (+37 more)

### Community 4 - "CollabBackend<B>"
Cohesion: 0.07
Nodes (30): Clone, HashSet, CollabBackend, CollabBackend<B>, CollabConfig, CollabHandle, connect_once(), device_id_from_token() (+22 more)

### Community 5 - "StorageBackend"
Cohesion: 0.07
Nodes (76): FnMut, CalendarEvent, CalendarTodo, Contact, contact_resources(), contact_round_trips_through_vcard(), contact_save_list_get_delete_over_storage(), delete_contact() (+68 more)

### Community 6 - "main.rs"
Cohesion: 0.06
Nodes (69): File, base(), Config, default_grpc_addr(), default_journal_retention_days(), default_max_message_size(), default_max_upload_bytes(), Mode (+61 more)

### Community 7 - "MetricsBackend<B>"
Cohesion: 0.16
Nodes (24): CollabClientMsg, CollabServerMsg, Cursor, LineOp, LineSnapshot, NoteLinesSnapshot, PresenceInfo, DateTime (+16 more)

### Community 8 - "ws_sync.rs"
Cohesion: 0.05
Nodes (59): String, SyncError, B, F, Result, Self, Vec, run_sync() (+51 more)

### Community 9 - "fs_backend.rs"
Cohesion: 0.06
Nodes (18): delete_for_unknown_sidecar_entity_leaves_a_tombstone_blocking_a_stale_create(), drain_sync(), fs_concurrent_note_tag_add_remove_converges(), fs_global_log_compacts_and_peer_converges(), fs_global_log_snapshot_covers_all_entity_types(), fs_note_log_compacts_and_still_converges(), fs_two_device_causal_sync(), fs_two_device_concurrent_edits_converge() (+10 more)

### Community 10 - "`{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}"
Cohesion: 0.25
Nodes (7): {{1. The concept / the model}}, {{2. How it works across the system}}, {{3. Guarantees and non-guarantees}}, {{4. Operational implications}}, Related documents, {{Title}} — {{one-line framing}}, Trade-offs & rejected alternatives

### Community 11 - "search.rs"
Cohesion: 0.15
Nodes (32): Database, apply_change(), denormalize(), empty_query_lists_by_recency_with_filters(), fts_match(), idx(), Index, index_note() (+24 more)

### Community 12 - "KeeplinServer<B>"
Cohesion: 0.07
Nodes (27): DeleteNotebookRequest, DeleteNotebookResponse, DeleteResourceRequest, DeleteResourceResponse, GetNotebookRequest, GetNotebookResponse, KeeplinServer<B>, Request (+19 more)

### Community 13 - "in_memory_backend"
Cohesion: 0.11
Nodes (37): add_and_list_note_tags(), add_note_tag_rejects_missing_or_deleted_ends(), apply_change_is_not_re_journaled(), backlinks_are_paginated(), concurrent_note_creates_all_succeed(), concurrent_reads_and_writes_make_progress(), create_and_read_note(), create_and_read_notebook() (+29 more)

### Community 14 - "Result"
Cohesion: 0.08
Nodes (27): AddNoteLinkRequest, AddNoteLinkResponse, AddNoteTagRequest, AddNoteTagResponse, CoreNote, GetNoteRequest, GetNoteResponse, note_to_proto() (+19 more)

### Community 15 - "server.rs"
Cohesion: 0.09
Nodes (33): CoreBookmark, CoreNotebook, CoreNoteLink, CoreTag, CreateNotebookRequest, CreateNotebookResponse, CreateTagRequest, CreateTagResponse (+25 more)

### Community 16 - "note_log.rs"
Cohesion: 0.14
Nodes (34): BTreeMap, causal_edit_after_delete_resurrects(), causal_update_wins_without_conflict(), compact_own_log(), compact_own_log_preserves_merge(), concurrent_edits_conflict_and_break_by_timestamp(), dominates(), entry() (+26 more)

### Community 17 - "LinkingBackend<B>"
Cohesion: 0.05
Nodes (93): BTreeSet, add_and_remove_manual_link(), add_manual_link(), alias_and_link_edits_reject_deleted_entities(), alias_conflicts(), alias_conflicts_lists_duplicates(), alias_index_tracks_deletes_and_renames(), AliasConflict (+85 more)

### Community 18 - "linking.rs"
Cohesion: 0.25
Nodes (8): Dependency graph (intra-crate), Design notes, Graph context, `{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}, Module map, Purpose, Related files, Startup / wiring

### Community 19 - "links.rs"
Cohesion: 0.15
Nodes (22): bookmark_re(), BookmarkRef, content_link_re(), DerivedBookmark, extracts_bookmarks_with_and_without_alias_in_order(), extracts_content_links_excluding_bookmarks(), LinkSource, LinkTarget (+14 more)

### Community 20 - "Change"
Cohesion: 0.07
Nodes (39): D, strip_resource_blob(), change_affects_aliases(), Change, de_notebook_id(), new_id(), now(), pre_ordering_note_msgpack_round_trips() (+31 more)

### Community 21 - "`storage/db.rs` — DbBackend (LibSQL + WebSocket storage)"
Cohesion: 0.09
Nodes (22): `apply_change` — all 13 variants, `apply_change` does **not** journal — and that is deliberate, Change journal — `record_change`, Connection and authentication, `data` column for resources, Database schema, `DbBackend::new(db_path, server_url, auth_token) -> Result<Self, StorageError>`, Design notes (+14 more)

### Community 22 - "Message types"
Cohesion: 0.09
Nodes (21): `Bookmark`, Linking & references, Message types, `Note`, `Notebook`, Notebooks, `NoteLink`, Notes (+13 more)

### Community 23 - "Vec"
Cohesion: 0.25
Nodes (8): Design notes, Graph context, Key types, {{Module-specific mechanism}}, `{{path/to/module.rs}}` — {{one-line purpose}}, Public API, Purpose, Related files

### Community 24 - "history.rs"
Cohesion: 0.13
Nodes (28): IntoIterator, Item, batch_revert_of_a_notebook_rolls_back_every_note(), fs(), note_history_lists_versions_newest_first(), revert_note(), revert_notebook(), revert_notebook_notes_to() (+20 more)

### Community 25 - "migrate"
Cohesion: 0.25
Nodes (8): Coverage gaps, {{Feature area}}, Fixtures and helpers, Graph context, Related files, Test cases, `{{tests/file.rs}}` — {{what it tests}}, What is tested

### Community 26 - "gRPC methods"
Cohesion: 0.11
Nodes (18): Conversion helpers (module-private), Data flow (example: `CreateNote`), Design notes, Graph context, gRPC methods, `KeeplinServer::new(backend: B) -> Self`, Key types, Linking & references RPCs (+10 more)

### Community 27 - "Response"
Cohesion: 0.15
Nodes (12): CoreResource, CreateResourceRequest, CreateResourceResponse, DeleteNoteRequest, DeleteNoteResponse, GetResourceRequest, GetResourceResponse, resource_to_proto() (+4 more)

### Community 28 - "`models.rs` — domain data types"
Cohesion: 0.11
Nodes (17): `Change` enum — all 13 variants, Design notes, `fn new_id() -> Uuid`, `fn now() -> DateTime<Utc>`, Graph context, Key types, `Note`, `Notebook` (+9 more)

### Community 29 - ".update_notebook"
Cohesion: 0.16
Nodes (13): CreateNoteRequest, CreateNoteResponse, ensure_not_deleted(), parse_optional_dt(), DateTime, Fn, Option, Utc (+5 more)

### Community 30 - "Status"
Cohesion: 0.13
Nodes (12): Status, stage_to_proto(), storage_err(), ListBacklinksRequest, ListBacklinksResponse, ListNotesInNotebookRequest, ListNotesInNotebookResponse, RemoveNoteTagRequest (+4 more)

### Community 31 - "Note"
Cohesion: 0.29
Nodes (7): Configuration / key reference, Graph context, Notes & gotchas, `{{path/to/file}}` — {{what it configures / generates}}, Purpose, Related files, What it {{generates | defines | runs}}

### Community 32 - "`storage/fs.rs` — FsBackend (filesystem storage)"
Cohesion: 0.12
Nodes (15): `apply_change`, Atomic write pattern, Backward compatibility, Concurrency — `note_write_lock`, Design notes, Directory layout, Format migrations (`ensure_format_version`, `FORMAT_VERSION = 5`), Global journal & format — `LogEntry`, `SyncState` (+7 more)

### Community 33 - "`src/main.rs` — daemon entry point"
Cohesion: 0.12
Nodes (15): Command-line interface, Design notes, Environment variables, `fn validate_basic_auth(req: tonic::Request<()>, expected_user: Option<&str>, expected_pass: Option<&str>) -> Result<tonic::Request<()>, tonic::Status>`, Graph context, Key types, `run_server`, `migrate` subcommand — `run_migrate(from, to)` (+7 more)

### Community 34 - "`encryption.rs` — transparent at-rest encryption"
Cohesion: 0.13
Nodes (14): Data flow, Design notes, `EncryptedBackend::new(inner: B, password: &str, salt: &[u8]) -> Result<Self, StorageError>`, `encryption.rs` — transparent at-rest encryption, Encryption scheme, Fields stored in plaintext (by design), Fields that are encrypted, `fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], StorageError>` (module-private) (+6 more)

### Community 35 - "run_sync"
Cohesion: 0.33
Nodes (6): Documentation templates, House style, Placeholders in the templates, The convention in one sentence, The two-layer navigation model, Which template to use

### Community 36 - "`ordering.rs` — the Inbox, pinning, manual ordering, and starring"
Cohesion: 0.13
Nodes (14): Design notes, Graph context, How ordering syncs, Invariants & edge cases, Key constants, Listing starred notes, `ordering.rs` — the Inbox, pinning, manual ordering, and starring, Pinning (+6 more)

### Community 37 - "Test cases"
Cohesion: 0.13
Nodes (14): Coverage gaps, Device and sync state, Fixtures and helpers, Graph context, Notebooks, Notes, Pinning, ordering & starring, Related files (+6 more)

### Community 38 - "enc_backend"
Cohesion: 0.25
Nodes (12): enc_backend(), list_notes_decrypts_all(), list_notes_paginates_and_decrypts_each_page(), note_round_trips(), note_tag_relation_preserved(), notebook_round_trips(), resource_data_stored_encrypted(), resource_round_trips() (+4 more)

### Community 39 - "Test cases"
Cohesion: 0.13
Nodes (14): Coverage gaps, Device and sync state, Durability, hygiene & multi-device safety, Fixtures and helpers, Graph context, Notebooks, Notes, Ordering, starring & the note index (+6 more)

### Community 40 - "`src/config.rs` — daemon configuration"
Cohesion: 0.13
Nodes (14): Auth is all-or-nothing (`auth_enabled` / `validate_auth`), `Config` fields, `Config::from_file(path: impl AsRef<Path>) -> anyhow::Result<Self>`, Design notes, Environment variable overrides, Graph context, Key types, `Mode` variants (+6 more)

### Community 41 - "Option"
Cohesion: 0.33
Nodes (5): Behaviour, Purpose, Refresh procedure after large refactors, Related files, `scripts/check-docs.sh` — contractual-docs CI check

### Community 42 - "`collab/mod.rs` — client of the keeplin-srv collaborative channel"
Cohesion: 0.14
Nodes (13): `collab/mod.rs` — client of the keeplin-srv collaborative channel, Design notes, Echo suppression, Graph context, Key types, Pending push: reconcile on `Welcome`, don't clobber, Public API, Purpose (+5 more)

### Community 43 - "Public API"
Cohesion: 0.14
Nodes (13): Design notes, `fn compact_own_log(log: &[NoteLogEntry]) -> Vec<NoteLogEntry>`, `fn dominates(a: &VersionVector, b: &VersionVector) -> bool`, `fn increment(vv: &mut VersionVector, device: &str)`, `fn join(a, b) -> VersionVector`, `fn merge(logs: &[Vec<NoteLogEntry>]) -> Merged`, `fn resolve(local: (vv, ts, device), incoming: (vv, ts, device)) -> Winner`, Graph context (+5 more)

### Community 44 - "Keeplin"
Cohesion: 0.14
Nodes (14): Bookmarks & links, Configuration reference, Development, Features, gRPC API, Keeplin, License, Migrating between backends (+6 more)

### Community 45 - "Keeplin — Architecture overview"
Cohesion: 0.17
Nodes (12): 1. What Keeplin is, 2. The domain model (`keeplin-core/src/models.rs`), 3. The storage trait and the two backends, 4. The decorator stack — the key idea, 5. Encryption (`keeplin-core/src/encryption.rs`), 6½. Organisation — the Inbox, pinning, ordering, starring (`keeplin-core/src/ordering.rs`), 6. Bookmarks and links (`links.rs` + `linking.rs`), 7½. History and revert (`keeplin-core/src/history.rs`) (+4 more)

### Community 46 - "`linking.rs` — `LinkingBackend` decorator + reference resolution"
Cohesion: 0.15
Nodes (12): Concurrency, Design notes, Free helper functions (called by the surfaces), Graph context, `linking.rs` — `LinkingBackend` decorator + reference resolution, Placement in the decorator stack, Purpose, Related files (+4 more)

### Community 47 - "`rest.rs` — REST/JSON API + WebSocket feed (axum)"
Cohesion: 0.15
Nodes (12): Auth middleware, Collaborative presence (`GET /notes/:id/presence`, `PUT /notes/:id/cursor`), Endpoints, Error mapping (`ApiError`), Graph context, State, Purpose, Related files (+4 more)

### Community 48 - "compat.rs"
Cohesion: 0.29
Nodes (9): compatible_with(), Handshake, incompatible_message(), incompatible_message_names_the_side_to_upgrade(), negotiate(), Client, String, Vec (+1 more)

### Community 50 - "`links.rs` — bookmark & link types and pure parsing"
Cohesion: 0.17
Nodes (11): Design notes, Graph context, `links.rs` — bookmark & link types and pure parsing, Parsed / grammar types, Persisted types (fields on `Note`), Pure functions, Purpose, Reference grammar (+3 more)

### Community 51 - "`migrate.rs` — one-shot state copy between backends"
Cohesion: 0.17
Nodes (11): `fn migrate(src: &dyn StorageBackend, dst: &dyn StorageBackend) -> Result<MigrationReport>`, Graph context, Helper, How it's invoked, `migrate.rs` — one-shot state copy between backends, Public API, Purpose, Related files (+3 more)

### Community 52 - "`storage/backend.rs` — the `StorageBackend` supertrait"
Cohesion: 0.17
Nodes (11): Design notes, Graph context, `HistoryRepository`, Notebooks, Tags, Notes (`NoteRepository`), Purpose, Related files, Resources (`ResourceRepository`) (+3 more)

### Community 53 - "device"
Cohesion: 0.33
Nodes (9): create_propagates_between_devices(), db_concurrent_equal_timestamp_edits_converge(), db_concurrent_note_tag_add_remove_converges(), db_concurrent_notebook_edits_converge(), db_resource_delete_propagates_and_converges(), db_stale_delete_does_not_override_newer_edit(), db_stale_update_does_not_resurrect_tombstone(), device() (+1 more)

### Community 54 - "`Cargo.toml` — workspace root"
Cohesion: 0.20
Nodes (9): `Cargo.toml` — workspace root, Crate purpose, Dev / build dependencies (shared), Related files, Release profile, Resolver, Runtime dependencies (shared), Workspace-level shared packages (+1 more)

### Community 55 - "`.github/workflows/ci.yml` — CI pipeline"
Cohesion: 0.20
Nodes (9): Caching strategy, Environment variables, `.github/workflows/ci.yml` — CI pipeline, Jobs, Notes, Purpose, Related files, `test` — Check, Test & Lint (+1 more)

### Community 56 - "`collab/state.rs` — client line state and body↔lines translation"
Cohesion: 0.18
Nodes (10): `apply` — trusting the server, `collab/state.rs` — client line state and body↔lines translation, Design notes, `diff_body` — the load-bearing algorithm, Graph context, Helpers, Key types, Public API (+2 more)

### Community 57 - "`history.rs` — change history reads + forward-revert"
Cohesion: 0.18
Nodes (10): "As of" semantics, Forward revert (non-destructive), Graph context, `history.rs` — change history reads + forward-revert, Public API, Purpose, Related files, Retention (+2 more)

### Community 58 - "`sync/engine.rs` — SyncEngine"
Cohesion: 0.18
Nodes (10): `async fn sync(&self) -> Result<Vec<Change>, SyncError>`, Data flow, Design notes, Graph context, Key types, Public API, Purpose, Related files (+2 more)

### Community 59 - "auth.rs"
Cohesion: 0.20
Nodes (4): basic(), Option, String, verify_basic()

### Community 60 - "`scripts/build.sh` — cross-compilation script"
Cohesion: 0.20
Nodes (9): Arguments, Environment variables, Notes, Prerequisites, Purpose, Related files, `scripts/build.sh` — cross-compilation script, Steps (+1 more)

### Community 61 - "`error.rs` — error types"
Cohesion: 0.20
Nodes (9): Design notes, `error.rs` — error types, `From` conversions, Graph context, Key types, Purpose, Related files, `StorageError` variants (+1 more)

### Community 62 - "`interop.rs` — vCard & iCalendar format compatibility"
Cohesion: 0.20
Nodes (9): API, Design notes, Format handling, Graph context, `interop.rs` — vCard & iCalendar format compatibility, Purpose, Related files, Typed storage over resources (+1 more)

### Community 63 - "mod.rs"
Cohesion: 0.28
Nodes (5): chrono::DateTime<chrono::Utc>, lexicographic_order_matches_chronological_even_mixed_with_old_format(), String, sortable_rfc3339_has_fixed_shape(), SortableRfc3339

### Community 64 - "`storage/mod.rs` — storage module root"
Cohesion: 0.20
Nodes (9): Design notes, Graph context, Module map, Page-size clamping — `effective_page_size`, Purpose, Re-exports, Related files, `SortableRfc3339` — fixed-precision timestamps for text comparison (+1 more)

### Community 65 - "`auth.rs` — shared HTTP Basic authentication"
Cohesion: 0.20
Nodes (9): `auth.rs` — shared HTTP Basic authentication, Design notes, `fn verify_basic(header: Option<&str>, expected_user: &str, expected_pass: &str) -> bool`, Graph context, How the surfaces use it, Public function, Purpose, Related files (+1 more)

### Community 66 - "`event_backend.rs` — `EventBackend` change-publishing decorator"
Cohesion: 0.20
Nodes (9): Construction, Delivery semantics, Design notes, `event_backend.rs` — `EventBackend` change-publishing decorator, Graph context, Placement in the stack, Purpose, Related files (+1 more)

### Community 67 - "`search.rs` — daemon full-text search"
Cohesion: 0.20
Nodes (9): Design notes, Graph context, How the index stays current, Key types, Purpose, Query building, Related files, `search.rs` — daemon full-text search (+1 more)

### Community 68 - "Security"
Cohesion: 0.25
Nodes (5): Credentials and TLS, Known limitations, Reporting vulnerabilities, Security, Threat model

### Community 69 - "`.cargo/config.toml` — workspace Cargo configuration"
Cohesion: 0.25
Nodes (7): Build profiles do **not** belong here, `.cargo/config.toml` — workspace Cargo configuration, Notes, Purpose, Related files, Sections, `[target.<triple>]` — cross-compilation (commented out)

### Community 70 - "`Cargo.toml` — keeplin-core"
Cohesion: 0.25
Nodes (7): Build-time notes, `Cargo.toml` — keeplin-core, Crate purpose, Dev / build dependencies, Feature flags, Related files, Runtime dependencies

### Community 71 - "`collab/protocol.rs` — collaborative channel wire types"
Cohesion: 0.22
Nodes (8): `collab/protocol.rs` — collaborative channel wire types, Design notes, Graph context, Key types, Message flow, Purpose, Related files, The op model

### Community 72 - "`compat.rs` — keeplin-srv protocol/capability handshake (`GET /version`)"
Cohesion: 0.22
Nodes (8): `compat.rs` — keeplin-srv protocol/capability handshake (`GET /version`), Design notes, Graph context, Key types, Public API, Purpose, Related files, The three-way contract (identical at both connect points)

### Community 73 - "`tests/encryption.rs` — EncryptedBackend integration tests"
Cohesion: 0.22
Nodes (8): Coverage gaps, Design notes on the tests, Fixtures and helpers, Graph context, Related files, Test cases, `tests/encryption.rs` — EncryptedBackend integration tests, What is tested

### Community 74 - "`Cargo.toml` — keeplin-daemon"
Cohesion: 0.25
Nodes (7): Build-time notes, `Cargo.toml` — keeplin-daemon, Crate purpose, Dev / build dependencies, Feature flags, Related files, Runtime dependencies

### Community 75 - "`src/proto.rs` — generated Protocol Buffers code"
Cohesion: 0.22
Nodes (8): Code generation, Design notes, Graph context, Lint suppressions, Module structure, Purpose, Related files, `src/proto.rs` — generated Protocol Buffers code

### Community 76 - "[Unreleased]"
Cohesion: 0.29
Nodes (6): [0.1.0], Added, Changed, Changelog, Documentation, [Unreleased]

### Community 77 - "finding.md"
Cohesion: 0.29
Nodes (6): Context, Impact, Problem, Severity, Suggested fix / options, Where

### Community 78 - "`lib.rs` — keeplin-core crate root"
Cohesion: 0.25
Nodes (7): Dependency graph (intra-crate), Design notes, Graph context, `lib.rs` — keeplin-core crate root, Module map, Purpose, Related files

### Community 79 - "`sync/mod.rs` — sync module root"
Cohesion: 0.25
Nodes (7): Design notes, Graph context, Module map, Purpose, Re-exports, Related files, `sync/mod.rs` — sync module root

### Community 80 - "`tests/migrate.rs` — cross-backend migration tests"
Cohesion: 0.25
Nodes (7): Coverage gaps, Fixtures and helpers, Graph context, Related files, Test cases, `tests/migrate.rs` — cross-backend migration tests, What is tested

### Community 81 - "`build.rs` — keeplin-daemon build script"
Cohesion: 0.25
Nodes (7): `build.rs` — keeplin-daemon build script, Build-time notes, Configuration, Graph context, Purpose, Related files, What it generates

### Community 82 - "`keeplin-daemon/src/metrics.rs` — operational metrics"
Cohesion: 0.25
Nodes (7): Endpoints (served by `rest.rs`), Exported series, Graph context, `keeplin-daemon/src/metrics.rs` — operational metrics, Purpose, Related files, Why a decorator (and where it sits)

### Community 83 - "Quick start"
Cohesion: 0.29
Nodes (7): Build, Configure, Prerequisites, Protocol compatibility with keeplin-srv, Quick start, Run, Run a sync server (server mode)

### Community 84 - "`tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e)"
Cohesion: 0.29
Nodes (6): Fixtures and helpers, Graph context, Related files, Test cases, `tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e), What is tested

### Community 85 - "`tests/sync.rs` — cross-device change-propagation tests"
Cohesion: 0.29
Nodes (6): Fixtures and helpers, Graph context, Related files, Test cases, `tests/sync.rs` — cross-device change-propagation tests, What is tested

### Community 86 - "`tests/version_handshake.rs` — startup protocol handshake tests"
Cohesion: 0.29
Nodes (6): Fixtures and helpers, Graph context, Related files, Test cases, `tests/version_handshake.rs` — startup protocol handshake tests, What is tested

### Community 87 - "`tests/ws_sync.rs` — end-to-end WebSocket sync test"
Cohesion: 0.29
Nodes (6): Fixtures and helpers, Graph context, Related files, Test cases, `tests/ws_sync.rs` — end-to-end WebSocket sync test, What is tested

### Community 88 - "main"
Cohesion: 0.40
Nodes (4): Box, main(), Error, Result

### Community 89 - "Design decisions"
Cohesion: 0.40
Nodes (5): Conflict resolution is unified on version vectors, Design decisions, Multi-device encryption constraint, Resource deletion, Sync delivery guarantee

### Community 90 - "Architecture"
Cohesion: 0.50
Nodes (4): Architecture, Backups, Multi‑device setup with Syncthing, Storage models

### Community 91 - "Encryption"
Cohesion: 0.50
Nodes (4): Collaborative (server) mode stores note title/body in cleartext on the server, Encrypted at rest, Encryption, Stored in plaintext by design

## Knowledge Gaps
- **501 isolated node(s):** `EpochHeader`, `build.sh script`, `check-docs.sh script`, `Purpose`, `Build profiles do **not** belong here` (+496 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `StorageError` connect `StorageError` to `FsBackend`, `rest.rs`, `DbBackend`, `CollabBackend<B>`, `StorageBackend`, `ws_sync.rs`, `LinkingBackend<B>`, `Change`, `history.rs`, `.update_notebook`, `Status`?**
  _High betweenness centrality (0.202) - this node is a cross-community bridge._
- **Why does `Note` connect `LinkingBackend<B>` to `FsBackend`, `rest.rs`, `StorageError`, `DbBackend`, `CollabBackend<B>`, `StorageBackend`, `search.rs`, `Result`, `note_log.rs`, `links.rs`, `Change`, `history.rs`?**
  _High betweenness centrality (0.100) - this node is a cross-community bridge._
- **Why does `DbBackend` connect `DbBackend` to `ws_sync.rs`, `in_memory_backend`, `device`, `StorageBackend`?**
  _High betweenness centrality (0.082) - this node is a cross-community bridge._
- **What connects `EpochHeader`, `build.sh script`, `check-docs.sh script` to the rest of the system?**
  _501 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `FsBackend` be split into smaller, more focused modules?**
  _Cohesion score 0.05536435732455484 - nodes in this community are weakly interconnected._
- **Should `rest.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.060526709561574146 - nodes in this community are weakly interconnected._
- **Should `StorageError` be split into smaller, more focused modules?**
  _Cohesion score 0.05528123658222413 - nodes in this community are weakly interconnected._