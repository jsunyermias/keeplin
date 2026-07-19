# Graph Report - keeplin  (2026-07-19)

## Corpus Check
- 100 files · ~240,546 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 3158 nodes · 7671 edges · 135 communities (128 shown, 7 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 24 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `e3525a7a`
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
- Notebook
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
- AliasIndex
- LineOp
- Note
- collab_client.rs
- .update_note
- mod tests
- mod tests
- impl LinkingBackend
- impl AliasIndex
- `Cargo.toml` — workspace root
- .get_changes_since
- impl NoteRepository for DbBackend
- impl TagRepository for DbBackend
- impl SyncBackend for DbBackend
- impl TagRepository for FsBackend
- impl SyncBackend for FsBackend
- impl NoteRepository for FsBackend
- mod tests
- `Cargo.toml` — keeplin-core
- `Cargo.toml` — keeplin-daemon
- mod migration_tests
- CollabHandle
- impl CalendarTodo
- impl NotebookRepository for DbBackend
- impl ResourceRepository for DbBackend
- impl NotebookRepository for FsBackend
- impl ResourceRepository for FsBackend
- impl DbBackend (server history)
- impl CalendarEvent
- impl PageCollector
- impl Contact
- impl HistoryRepository for DbBackend
- impl HistoryRepository for FsBackend
- impl KeeplinServer
- .set_notebook_alias

## God Nodes (most connected - your core abstractions)
1. `StorageError` - 406 edges
2. `Note` - 151 edges
3. `FsBackend` - 124 edges
4. ``rest.rs` — the REST/JSON + WebSocket surface` - 116 edges
5. `Shared` - 110 edges
6. `DbBackend` - 100 edges
7. `Notebook` - 67 edges
8. `ApiError` - 65 edges
9. `StorageBackend` - 64 edges
10. `EncryptedBackend<B>` - 56 edges

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

## Communities (135 total, 7 thin omitted)

### Community 0 - "FsBackend"
Cohesion: 0.06
Nodes (62): BinaryHeap, Eq, T, atomic_write(), compaction_declines_on_unreadable_sidecar_and_resumes_after_repair(), concurrent_same_note_updates_keep_every_log_entry(), corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it(), default_entity_type() (+54 more)

### Community 1 - "rest.rs"
Cohesion: 0.06
Nodes (158): Body, Bytes, HeaderMap, IntoResponse, Json, Client, HashMap, Shared (+150 more)

### Community 2 - "StorageError"
Cohesion: 0.10
Nodes (20): backend(), counts_applied_sync_changes(), counts_operations_and_errors(), http_status_buckets(), MetricsBackend<B>, Arc, AtomicU64, B (+12 more)

### Community 3 - "DbBackend"
Cohesion: 0.08
Nodes (52): AtomicBool, Error, From, StorageError, Self, Bookmark, DateTime, EntityVersion (+44 more)

### Community 4 - "CollabBackend<B>"
Cohesion: 0.18
Nodes (8): Clone, CollabBackend<B>, device_id_from_token(), Option, Result, String, Uuid, Vec

### Community 5 - "StorageBackend"
Cohesion: 0.08
Nodes (68): FnMut, IntoIterator, Item, contact_resources(), contact_round_trips_through_vcard(), contact_save_list_get_delete_over_storage(), delete_contact(), delete_event() (+60 more)

### Community 6 - "main.rs"
Cohesion: 0.06
Nodes (69): File, base(), default_grpc_addr(), default_journal_retention_days(), default_max_message_size(), default_max_upload_bytes(), multiple_issues_accumulate(), network_grpc_without_auth_is_flagged() (+61 more)

### Community 7 - "MetricsBackend<B>"
Cohesion: 0.25
Nodes (9): bump(), merge_into(), HashMap, NoteLines, Self, String, Uuid, Vec (+1 more)

### Community 8 - "ws_sync.rs"
Cohesion: 0.07
Nodes (47): SyncError, String, B, F, SyncEngine, SyncStage, Result, Self (+39 more)

### Community 9 - "fs_backend.rs"
Cohesion: 0.06
Nodes (17): drain_sync(), fs_concurrent_note_tag_add_remove_converges(), fs_global_log_compacts_and_peer_converges(), fs_global_log_snapshot_covers_all_entity_types(), fs_note_log_compacts_and_still_converges(), fs_two_device_causal_sync(), fs_two_device_concurrent_edits_converge(), note_index_reflects_changes_pulled_from_a_peer() (+9 more)

### Community 10 - "`{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}"
Cohesion: 0.05
Nodes (34): Configuration / key reference, Graph context, Notes & gotchas, `{{path/to/file}}` — {{what it configures / generates}}, Purpose, Related files, What it {{generates | defines | runs}}, Dependency graph (intra-crate) (+26 more)

### Community 11 - "search.rs"
Cohesion: 0.15
Nodes (32): Database, apply_change(), denormalize(), empty_query_lists_by_recency_with_filters(), fts_match(), idx(), index_note(), indexes_from_rebuild_and_the_event_stream() (+24 more)

### Community 12 - "KeeplinServer<B>"
Cohesion: 0.07
Nodes (27): DeleteNotebookRequest, DeleteNotebookResponse, DeleteResourceRequest, DeleteResourceResponse, GetNotebookRequest, GetNotebookResponse, GetTagRequest, GetTagResponse (+19 more)

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
Nodes (34): causal_edit_after_delete_resurrects(), causal_update_wins_without_conflict(), compact_own_log(), compact_own_log_preserves_merge(), concurrent_edits_conflict_and_break_by_timestamp(), dominates(), entry(), increment() (+26 more)

### Community 17 - "LinkingBackend<B>"
Cohesion: 0.12
Nodes (18): alias_conflicts(), backlinks(), collect_notebooks(), group_conflicts(), group_note_conflicts(), LinkingBackend<B>, read_live_notebook(), DateTime (+10 more)

### Community 18 - "linking.rs"
Cohesion: 0.10
Nodes (13): Aes256Gcm, derive_key(), EncryptedBackend<B>, B, DateTime, EncryptedBackend, Option, Result (+5 more)

### Community 19 - "links.rs"
Cohesion: 0.15
Nodes (22): bookmark_re(), content_link_re(), extracts_bookmarks_with_and_without_alias_in_order(), extracts_content_links_excluding_bookmarks(), parse_bookmarks(), parse_content_links(), parse_link_ref(), parses_one_two_three_segments() (+14 more)

### Community 20 - "Change"
Cohesion: 0.08
Nodes (40): D, strip_resource_blob(), change_affects_aliases(), de_notebook_id(), new_id(), now(), pre_ordering_note_msgpack_round_trips(), DateTime (+32 more)

### Community 21 - "`storage/db.rs` — DbBackend (LibSQL + WebSocket storage)"
Cohesion: 0.09
Nodes (21): Coverage checklist, fn assoc_data, fn assoc_from_data, fn bookmarks_to_json, fn build_page, fn http_base_of, fn json_to_bookmarks, fn json_to_links (+13 more)

### Community 22 - "Message types"
Cohesion: 0.09
Nodes (21): `Bookmark`, Linking & references, Message types, `Note`, `Notebook`, Notebooks, `NoteLink`, Notes (+13 more)

### Community 23 - "Vec"
Cohesion: 0.04
Nodes (56): fn append_log, fn append_note_op, fn apply_format_migration, fn assoc_incoming_wins, fn build_global_snapshot, fn build_note_index, fn collect_advanced_notes, fn compact_global_log_locked (+48 more)

### Community 24 - "history.rs"
Cohesion: 0.27
Nodes (17): batch_revert_of_a_notebook_rolls_back_every_note(), fs(), note_history_lists_versions_newest_first(), revert_note(), revert_notebook(), revert_notebook_notes_to(), revert_notes_to(), revert_restores_an_earlier_version() (+9 more)

### Community 25 - "migrate"
Cohesion: 0.05
Nodes (40): fn add_note_link, fn add_note_tag, fn create_note, fn create_notebook, fn create_resource, fn create_tag, fn delete_note, fn delete_notebook (+32 more)

### Community 26 - "gRPC methods"
Cohesion: 0.08
Nodes (24): Coverage checklist, fn bookmark_to_proto, fn ensure_not_deleted, fn link_source_str, fn note_to_proto, fn notebook_to_proto, fn notelink_to_proto, fn parse_optional_dt (+16 more)

### Community 27 - "Response"
Cohesion: 0.15
Nodes (12): CoreResource, CreateResourceRequest, CreateResourceResponse, GetResourceRequest, GetResourceResponse, resource_to_proto(), Response, RemoveNoteTagRequest (+4 more)

### Community 28 - "`models.rs` — domain data types"
Cohesion: 0.07
Nodes (26): Coverage checklist, DEFAULT_SORT_KEY, fn de_notebook_id, fn effective_sort_key, fn new, fn new, fn new, fn new (+18 more)

### Community 29 - ".update_notebook"
Cohesion: 0.16
Nodes (13): CreateNoteRequest, CreateNoteResponse, ensure_not_deleted(), parse_optional_dt(), DateTime, Fn, Option, Utc (+5 more)

### Community 30 - "Status"
Cohesion: 0.13
Nodes (12): DeleteNoteRequest, DeleteNoteResponse, Status, stage_to_proto(), storage_err(), ListNotesRequest, ListNotesResponse, ListStarredNotesRequest (+4 more)

### Community 31 - "Note"
Cohesion: 0.06
Nodes (35): fn add_column_if_missing, fn apply_migration, fn assoc_incoming_wins, fn assoc_meta, fn begin, fn commit, fn connect_ws, fn current_meta (+27 more)

### Community 32 - "`storage/fs.rs` — FsBackend (filesystem storage)"
Cohesion: 0.05
Nodes (36): Coverage checklist, fn apply, fn atomic_write, fn default_entity_type, fn from_note, fn fs_assoc_from_data, fn fs_assoc_value, fn fs_tombstone_from_data (+28 more)

### Community 33 - "`src/main.rs` — daemon entry point"
Cohesion: 0.05
Nodes (38): Coverage checklist, fn acquire_store_lock, fn auth_bearer_scheme_rejected, fn auth_malformed_base64_rejected, fn auth_missing_header_rejected, fn auth_no_colon_in_credentials_rejected, fn auth_not_configured_allows_all, fn auth_password_containing_colon_works (+30 more)

### Community 34 - "`encryption.rs` — transparent at-rest encryption"
Cohesion: 0.07
Nodes (27): Coverage checklist, `encryption.rs` — transparent at-rest encryption decorator, fn dec_note, fn dec_notebook, fn dec_resource, fn dec_tag, fn decrypt_bytes, fn decrypt_str (+19 more)

### Community 35 - "run_sync"
Cohesion: 0.19
Nodes (31): add_and_remove_manual_link(), alias_conflicts_lists_duplicates(), alias_index_tracks_deletes_and_renames(), aliased(), backend(), bare_alias_resolves_globally_when_unique_else_scoped(), bookmark_alias_comes_from_the_body_title(), collect_notes() (+23 more)

### Community 36 - "`ordering.rs` — the Inbox, pinning, manual ordering, and starring"
Cohesion: 0.05
Nodes (39): Coverage checklist, fn a_same_notebook_edit_keeps_the_position, fn backend, fn create_placed, fn ensure_inbox, fn ensure_inbox_is_idempotent_and_fixed, fn inbox_top_insert_survives_underflow_by_resequencing, fn is_inbox (+31 more)

### Community 37 - "Test cases"
Cohesion: 0.05
Nodes (42): Coverage checklist, fn add_and_list_note_tags, fn add_note_tag_rejects_missing_or_deleted_ends, fn apply_change_is_not_re_journaled, fn backlinks_are_paginated, fn concurrent_note_creates_all_succeed, fn concurrent_reads_and_writes_make_progress, fn create_and_read_note (+34 more)

### Community 38 - "enc_backend"
Cohesion: 0.25
Nodes (12): enc_backend(), list_notes_decrypts_all(), list_notes_paginates_and_decrypts_each_page(), note_round_trips(), note_tag_relation_preserved(), notebook_round_trips(), resource_data_stored_encrypted(), resource_round_trips() (+4 more)

### Community 39 - "Test cases"
Cohesion: 0.04
Nodes (49): Coverage checklist, fn add_and_list_note_tags, fn add_note_tag_rejects_missing_or_deleted_ends, fn backlinks_default_scan_is_paginated, fn create_and_read_note, fn create_and_read_notebook, fn create_and_read_resource, fn create_and_read_tag (+41 more)

### Community 40 - "`src/config.rs` — daemon configuration"
Cohesion: 0.07
Nodes (29): `config.rs` — daemon runtime configuration, Coverage checklist, fn auth_enabled, fn base, fn default_grpc_addr, fn default_journal_retention_days, fn default_max_message_size, fn default_max_upload_bytes (+21 more)

### Community 41 - "Option"
Cohesion: 0.22
Nodes (8): Behaviour, Known caveat, Purpose, Refresh procedure after large refactors, Related files, `scripts/check-docs.sh` — contractual-docs CI check, What it checks, What it deliberately does NOT verify

### Community 42 - "`collab/mod.rs` — client of the keeplin-srv collaborative channel"
Cohesion: 0.04
Nodes (44): `collab/mod.rs` — client of the keeplin-srv collaborative channel, Coverage checklist, DISCOVER_PAGE_SIZE, fn apply_from_server, fn auth, fn connect_once, fn device_id_from_token, fn discover_and_join (+36 more)

### Community 43 - "Public API"
Cohesion: 0.06
Nodes (33): Coverage checklist, fn causal_edit_after_delete_resurrects, fn causal_update_wins_without_conflict, fn compact_own_log, fn compact_own_log_preserves_merge, fn concurrent_edits_conflict_and_break_by_timestamp, fn dominates, fn entry (+25 more)

### Community 44 - "Keeplin"
Cohesion: 0.14
Nodes (14): Bookmarks & links, Configuration reference, Development, Features, gRPC API, Keeplin, License, Migrating between backends (+6 more)

### Community 45 - "Keeplin — Architecture overview"
Cohesion: 0.17
Nodes (12): 1. What Keeplin is, 2. The domain model (`keeplin-core/src/models.rs`), 3. The storage trait and the two backends, 4. The decorator stack — the key idea, 5. Encryption (`keeplin-core/src/encryption.rs`), 6½. Organisation — the Inbox, pinning, ordering, starring (`keeplin-core/src/ordering.rs`), 6. Bookmarks and links (`links.rs` + `linking.rs`), 7½. History and revert (`keeplin-core/src/history.rs`) (+4 more)

### Community 46 - "`linking.rs` — `LinkingBackend` decorator + reference resolution"
Cohesion: 0.06
Nodes (32): Coverage checklist, fn add_manual_link, fn alias_conflicts, fn backlinks, fn change_affects_aliases, fn collect_notebooks, fn collect_notes, fn group_conflicts (+24 more)

### Community 47 - "`rest.rs` — REST/JSON API + WebSocket feed (axum)"
Cohesion: 0.02
Nodes (115): Coverage checklist, fn add_link, fn add_note_tag, fn auth_mw, fn batch_revert_notes_ep, fn create_note, fn create_notebook, fn create_resource (+107 more)

### Community 48 - "compat.rs"
Cohesion: 0.29
Nodes (9): compatible_with(), incompatible_message(), incompatible_message_names_the_side_to_upgrade(), negotiate(), Client, Handshake, ServerInfo, String (+1 more)

### Community 49 - "Notebook"
Cohesion: 0.20
Nodes (31): a_same_notebook_edit_keeps_the_position(), backend(), create_placed(), ensure_inbox(), ensure_inbox_is_idempotent_and_fixed(), inbox_top_insert_survives_underflow_by_resequencing(), is_inbox(), lowest_free_pinned_key() (+23 more)

### Community 50 - "`links.rs` — bookmark & link types and pure parsing"
Cohesion: 0.06
Nodes (30): Coverage checklist, fn bookmark_re, fn bookmark_ref_zero_is_alias, fn content_link_re, fn extracts_bookmarks_with_and_without_alias_in_order, fn extracts_content_links_excluding_bookmarks, fn from_raw, fn parse (+22 more)

### Community 51 - "`migrate.rs` — one-shot state copy between backends"
Cohesion: 0.22
Nodes (8): Coverage checklist, fn collect, fn migrate, Graph context, MigrationReport, `migrate.rs` — one-shot state copy between backends, Overview, PAGE

### Community 52 - "`storage/backend.rs` — the `StorageBackend` supertrait"
Cohesion: 0.11
Nodes (18): Coverage checklist, DEFAULT_HISTORY_LIMIT, fn from_effective_keys, fn paginate_notes, Graph context, impl NotebookSortProfile, impl StorageBackend for T, EntityVersion (+10 more)

### Community 53 - "device"
Cohesion: 0.33
Nodes (9): create_propagates_between_devices(), db_concurrent_equal_timestamp_edits_converge(), db_concurrent_note_tag_add_remove_converges(), db_concurrent_notebook_edits_converge(), db_resource_delete_propagates_and_converges(), db_stale_delete_does_not_override_newer_edit(), db_stale_update_does_not_resurrect_tombstone(), device() (+1 more)

### Community 54 - "`Cargo.toml` — workspace root"
Cohesion: 0.08
Nodes (25): fn alias_and_links_endpoints, fn alias_backlinks_and_resolve_endpoints, fn alias_conflicts_endpoint, fn auth_is_enforced_when_configured, fn call, fn contact_import_list_export_delete_endpoints, fn invalid_uuid_is_bad_request, fn linking_state (+17 more)

### Community 55 - "`.github/workflows/ci.yml` — CI pipeline"
Cohesion: 0.20
Nodes (9): Caching strategy, Environment variables, `.github/workflows/ci.yml` — CI pipeline, Jobs, Notes, Purpose, Related files, `test` — Check, Test & Lint (+1 more)

### Community 56 - "`collab/state.rs` — client line state and body↔lines translation"
Cohesion: 0.14
Nodes (13): `collab/state.rs` — client line state and body↔lines translation, Coverage checklist, fn apply, fn bump, fn diff_body, fn from_snapshot, fn live, fn materialize (+5 more)

### Community 57 - "`history.rs` — change history reads + forward-revert"
Cohesion: 0.11
Nodes (18): Coverage checklist, fn batch_revert_of_a_notebook_rolls_back_every_note, fn fs, fn note_history_lists_versions_newest_first, fn revert_note, fn revert_notebook, fn revert_notebook_notes_to, fn revert_notes_to (+10 more)

### Community 58 - "`sync/engine.rs` — SyncEngine"
Cohesion: 0.18
Nodes (10): Coverage checklist, fn new, fn run_sync, fn sync, Graph context, impl SyncEngine, SyncEngine, SyncStage (+2 more)

### Community 59 - "auth.rs"
Cohesion: 0.20
Nodes (4): basic(), Option, String, verify_basic()

### Community 60 - "`scripts/build.sh` — cross-compilation script"
Cohesion: 0.20
Nodes (9): Arguments, Environment variables, Notes, Prerequisites, Purpose, Related files, `scripts/build.sh` — cross-compilation script, Steps (+1 more)

### Community 61 - "`error.rs` — error types"
Cohesion: 0.22
Nodes (8): Coverage checklist, `error.rs` — error types, Graph context, impl From libsql for StorageError, impl From tungstenite for StorageError, StorageError, SyncError, Overview

### Community 62 - "`interop.rs` — vCard & iCalendar format compatibility"
Cohesion: 0.05
Nodes (38): Coverage checklist, fn contact_resources, fn delete_contact, fn delete_event, fn escape_text, fn event_resources, fn fold_line, fn format_dt (+30 more)

### Community 63 - "mod.rs"
Cohesion: 0.28
Nodes (5): chrono::DateTime<chrono::Utc>, lexicographic_order_matches_chronological_even_mixed_with_old_format(), String, sortable_rfc3339_has_fixed_shape(), SortableRfc3339

### Community 64 - "`storage/mod.rs` — storage module root"
Cohesion: 0.14
Nodes (13): Coverage checklist, DEFAULT_PAGE_SIZE, fn effective_page_size, fn effective_page_size_defaults_and_clamps, fn lexicographic_order_matches_chronological_even_mixed_with_old_format, fn sortable_rfc3339_has_fixed_shape, Graph context, impl SortableRfc3339 for DateTime Utc (+5 more)

### Community 65 - "`auth.rs` — shared HTTP Basic authentication"
Cohesion: 0.15
Nodes (12): `auth.rs` — shared HTTP Basic Authentication check, Coverage checklist, fn accepts_valid_credentials, fn basic, fn password_with_colons_works, fn rejects_empty_expected_credentials, fn rejects_wrong_password_user_and_missing_header, fn scheme_is_case_and_whitespace_tolerant (+4 more)

### Community 66 - "`event_backend.rs` — `EventBackend` change-publishing decorator"
Cohesion: 0.10
Nodes (19): Coverage checklist, `event_backend.rs` — `EventBackend` change-publishing decorator, fn backend, fn create_update_delete_emit_changes, fn failed_mutation_emits_nothing, fn new, fn publish, fn reads_do_not_emit_changes (+11 more)

### Community 67 - "`search.rs` — daemon full-text search"
Cohesion: 0.06
Nodes (34): Coverage checklist, fn apply_change, fn bit, fn clear, fn denormalize, fn empty_query_lists_by_recency_with_filters, fn fts_match, fn idx (+26 more)

### Community 68 - "Security"
Cohesion: 0.25
Nodes (5): Credentials and TLS, Known limitations, Reporting vulnerabilities, Security, Threat model

### Community 69 - "`.cargo/config.toml` — workspace Cargo configuration"
Cohesion: 0.25
Nodes (7): Build profiles do **not** belong here, `.cargo/config.toml` — workspace Cargo configuration, Notes, Purpose, Related files, Sections, `[target.<triple>]` — cross-compilation (commented out)

### Community 70 - "`Cargo.toml` — keeplin-core"
Cohesion: 0.09
Nodes (23): fn add_and_remove_manual_link, fn alias_and_link_edits_reject_deleted_entities, fn alias_conflicts_lists_duplicates, fn alias_index_tracks_deletes_and_renames, fn aliased, fn backend, fn bare_alias_resolves_globally_when_unique_else_scoped, fn bookmark_alias_comes_from_the_body_title (+15 more)

### Community 71 - "`collab/protocol.rs` — collaborative channel wire types"
Cohesion: 0.15
Nodes (12): `collab/protocol.rs` — collaborative channel wire types, Coverage checklist, Graph context, LineId, CollabClientMsg, CollabServerMsg, Cursor, LineOp (+4 more)

### Community 72 - "`compat.rs` — keeplin-srv protocol/capability handshake (`GET /version`)"
Cohesion: 0.14
Nodes (13): `compat.rs` — keeplin-srv protocol/capability handshake (`GET /version`), Coverage checklist, fn compatible_with, fn exact_match_is_compatible, fn incompatible_message, fn incompatible_message_names_the_side_to_upgrade, fn negotiate, Graph context (+5 more)

### Community 73 - "`tests/encryption.rs` — EncryptedBackend integration tests"
Cohesion: 0.11
Nodes (17): Coverage checklist, fn enc_backend, fn list_notes_decrypts_all, fn list_notes_paginates_and_decrypts_each_page, fn note_round_trips, fn note_tag_relation_preserved, fn notebook_round_trips, fn resource_data_stored_encrypted (+9 more)

### Community 74 - "`Cargo.toml` — keeplin-daemon"
Cohesion: 0.24
Nodes (14): HashSet, connect_once(), discover_and_join(), ensure_local(), handle_server_msg(), Arc, B, CollabBackend (+6 more)

### Community 75 - "`src/proto.rs` — generated Protocol Buffers code"
Cohesion: 0.40
Nodes (4): Coverage checklist, Graph context, Overview, `proto.rs` — generated Protocol Buffers / gRPC code

### Community 76 - "[Unreleased]"
Cohesion: 0.25
Nodes (7): [0.1.0], 2026-07 production-readiness audit follow-up, Added, Changed, Changelog, Documentation, [Unreleased]

### Community 77 - "finding.md"
Cohesion: 0.29
Nodes (6): Context, Impact, Problem, Severity, Suggested fix / options, Where

### Community 78 - "`lib.rs` — keeplin-core crate root"
Cohesion: 0.40
Nodes (4): Coverage checklist, Graph context, `lib.rs` — keeplin-core crate root, Overview

### Community 79 - "`sync/mod.rs` — sync module root"
Cohesion: 0.40
Nodes (4): Coverage checklist, Graph context, Overview, `sync/mod.rs` — sync module root

### Community 80 - "`tests/migrate.rs` — cross-backend migration tests"
Cohesion: 0.17
Nodes (11): Coverage checklist, fn assert_migrated, fn db, fn db_to_fs_round_trip, fn encrypted_fs_to_encrypted_db, fn fs_to_db_round_trip, fn seed, Graph context (+3 more)

### Community 81 - "`build.rs` — keeplin-daemon build script"
Cohesion: 0.40
Nodes (4): `build.rs` — keeplin-daemon build script, Coverage checklist, Graph context, main

### Community 82 - "`keeplin-daemon/src/metrics.rs` — operational metrics"
Cohesion: 0.06
Nodes (30): Coverage checklist, fn add_sync_applied, fn backend, fn counts_applied_sync_changes, fn counts_operations_and_errors, fn http_status_buckets, fn incr_error, fn incr_op (+22 more)

### Community 83 - "Quick start"
Cohesion: 0.29
Nodes (7): Build, Configure, Prerequisites, Protocol compatibility with keeplin-srv, Quick start, Run, Run a sync server (server mode)

### Community 84 - "`tests/collab_client.rs` — collaborative client tests (state machine + mock server e2e)"
Cohesion: 0.12
Nodes (15): Coverage checklist, fn client, fn created_note_body_survives_the_join_welcome, fn cursor_updates_flow_into_presence, fn diff_roundtrip_materializes_new_body, fn edits_travel_between_two_daemons, fn mock_server, fn ops_replay_identically_on_another_mirror (+7 more)

### Community 85 - "`tests/sync.rs` — cross-device change-propagation tests"
Cohesion: 0.13
Nodes (14): Coverage checklist, fn create_propagates_between_devices, fn db_concurrent_equal_timestamp_edits_converge, fn db_concurrent_note_tag_add_remove_converges, fn db_concurrent_notebook_edits_converge, fn db_resource_delete_propagates_and_converges, fn db_stale_delete_does_not_override_newer_edit, fn db_stale_update_does_not_resurrect_tombstone (+6 more)

### Community 86 - "`tests/version_handshake.rs` — startup protocol handshake tests"
Cohesion: 0.17
Nodes (11): Coverage checklist, fn collab_start_applies_the_same_rule, fn compatible_version_connects_and_primes_capabilities, fn db_path, fn fake_token, fn incompatible_version_fails_construction_loudly, fn missing_version_warns_and_continues, fn spawn_version_server (+3 more)

### Community 87 - "`tests/ws_sync.rs` — end-to-end WebSocket sync test"
Cohesion: 0.10
Nodes (20): Coverage checklist, fn a_404_history_endpoint_is_probed_only_once, fn device, fn epoch, fn failed_send_keeps_watermark_and_changes_are_resent_after_recovery, fn history_is_skipped_when_the_server_capability_is_absent, fn malformed_frame_does_not_abort_receive, fn note_create_syncs_between_two_devices (+12 more)

### Community 88 - "main"
Cohesion: 0.40
Nodes (4): Box, Error, main(), Result

### Community 89 - "Design decisions"
Cohesion: 0.40
Nodes (5): Conflict resolution is unified on version vectors, Design decisions, Multi-device encryption constraint, Resource deletion, Sync delivery guarantee

### Community 90 - "Architecture"
Cohesion: 0.50
Nodes (4): Architecture, Backups, Multi‑device setup with Syncthing, Storage models

### Community 91 - "Encryption"
Cohesion: 0.50
Nodes (4): Collaborative (server) mode stores note title/body in cleartext on the server, Encrypted at rest, Encryption, Stored in plaintext by design

### Community 93 - ".get_tag"
Cohesion: 0.22
Nodes (16): collect(), migrate(), F, MigrationReport, Result, Vec, assert_migrated(), db() (+8 more)

### Community 100 - "AliasIndex"
Cohesion: 0.15
Nodes (6): BTreeMap, BTreeSet, FnOnce, HashMap, AliasIndex, R

### Community 101 - "LineOp"
Cohesion: 0.37
Nodes (15): DateTime, CollabClientMsg, CollabServerMsg, Cursor, LineOp, LineSnapshot, NoteLinesSnapshot, PresenceInfo (+7 more)

### Community 102 - "Note"
Cohesion: 0.26
Nodes (7): add_manual_link(), alias_and_link_edits_reject_deleted_entities(), read_live_note(), remove_link(), Self, set_note_alias(), Note

### Community 103 - "collab_client.rs"
Cohesion: 0.27
Nodes (12): client(), created_note_body_survives_the_join_welcome(), cursor_updates_flow_into_presence(), edits_travel_between_two_daemons(), mock_server(), resource_blob_uploads_out_of_band_and_downloads_on_read(), Arc, SocketAddr (+4 more)

### Community 105 - "mod tests"
Cohesion: 0.14
Nodes (14): fn contact_round_trips_through_vcard, fn contact_save_list_get_delete_over_storage, fn event_round_trips_through_ics, fn event_round_trips_through_storage, fn fs, fn import_todo_creates_a_todo_note, fn missing_component_yields_none, fn multi_component_calendar_parses_every_event_and_todo (+6 more)

### Community 106 - "mod tests"
Cohesion: 0.15
Nodes (13): fn compaction_declines_on_unreadable_sidecar_and_resumes_after_repair, fn concurrent_same_note_updates_keep_every_log_entry, fn corrupt_assoc_state_is_weakest_priority_and_peer_state_recovers_it, fn detects_syncthing_conflict_copies_without_removing_them, fn failed_atomic_write_cleans_up_its_temp_file, fn fresh_store_is_stamped_current_version, fn list_notes_pages_match_full_walk, fn migrates_a_legacy_stamp_and_preserves_data (+5 more)

### Community 107 - "impl LinkingBackend"
Cohesion: 0.18
Nodes (11): fn ensure_notebook_alias_free, fn index_invalidate, fn index_remove_note, fn index_remove_notebook, fn index_upsert_note, fn index_upsert_notebook, fn new, fn prepare (+3 more)

### Community 108 - "impl AliasIndex"
Cohesion: 0.18
Nodes (11): fn from_snapshots, fn note_alias_taken, fn notebook_alias_taken, fn remove_note, fn remove_notebook, fn resolve_note_seg, fn resolve_notebook_seg, fn resolve_target (+3 more)

### Community 109 - "`Cargo.toml` — workspace root"
Cohesion: 0.20
Nodes (9): `Cargo.toml` — workspace root, Crate purpose, Dev / build dependencies (shared), Related files, Release profile, Resolver, Runtime dependencies (shared), Workspace-level shared packages (+1 more)

### Community 110 - ".get_changes_since"
Cohesion: 0.31
Nodes (4): is_note_change(), DateTime, ServerNote, Utc

### Community 111 - "impl NoteRepository for DbBackend"
Cohesion: 0.20
Nodes (10): fn create_note, fn delete_note, fn list_notes, fn list_notes_in_notebook, fn list_starred_notes, fn note_backlinks, fn notebook_sort_profile, fn read_note (+2 more)

### Community 112 - "impl TagRepository for DbBackend"
Cohesion: 0.22
Nodes (9): fn add_note_tag, fn create_tag, fn delete_tag, fn list_note_tags, fn list_tags, fn read_tag, fn remove_note_tag, fn update_tag (+1 more)

### Community 113 - "impl SyncBackend for DbBackend"
Cohesion: 0.22
Nodes (9): fn apply_change, fn get_changes_since, fn get_device_id, fn get_last_sync_time, fn prune_change_journal, fn receive_changes, fn send_changes, fn update_sync_time (+1 more)

### Community 114 - "impl TagRepository for FsBackend"
Cohesion: 0.22
Nodes (9): fn add_note_tag, fn create_tag, fn delete_tag, fn list_note_tags, fn list_tags, fn read_tag, fn remove_note_tag, fn update_tag (+1 more)

### Community 115 - "impl SyncBackend for FsBackend"
Cohesion: 0.22
Nodes (9): fn apply_change, fn get_changes_since, fn get_device_id, fn get_last_sync_time, fn prune_change_journal, fn receive_changes, fn send_changes, fn update_sync_time (+1 more)

### Community 116 - "impl NoteRepository for FsBackend"
Cohesion: 0.22
Nodes (9): fn create_note, fn delete_note, fn list_notes, fn list_notes_in_notebook, fn list_starred_notes, fn notebook_sort_profile, fn read_note, fn update_note (+1 more)

### Community 117 - "mod tests"
Cohesion: 0.22
Nodes (9): fn chunk_frame, fn meta_frame, fn server, fn update_notebook_and_tag_refresh_updated_at_server_side, fn update_rpcs_reject_soft_deleted_entities, fn upload_resource_assembles_chunks_in_order, fn upload_resource_enforces_the_cap, fn upload_resource_requires_metadata_first (+1 more)

### Community 118 - "`Cargo.toml` — keeplin-core"
Cohesion: 0.25
Nodes (7): Build-time notes, `Cargo.toml` — keeplin-core, Crate purpose, Dev / build dependencies, Feature flags, Related files, Runtime dependencies

### Community 119 - "`Cargo.toml` — keeplin-daemon"
Cohesion: 0.25
Nodes (7): Build-time notes, `Cargo.toml` — keeplin-daemon, Crate purpose, Dev / build dependencies, Feature flags, Related files, Runtime dependencies

### Community 120 - "mod migration_tests"
Cohesion: 0.29
Nodes (7): fn fresh_database_is_stamped_current_and_reopen_is_a_noop, fn migrates_a_pre_framework_database_without_losing_data, fn note_history_reads_this_devices_versions_newest_first, fn raw_conn, fn refuses_to_open_a_newer_schema, fn user_version, mod migration_tests

### Community 122 - "impl CalendarTodo"
Cohesion: 0.33
Nodes (6): fn apply_to_note, fn from_ics, fn from_ics_all, fn from_note, fn to_ics, impl CalendarTodo

### Community 123 - "impl NotebookRepository for DbBackend"
Cohesion: 0.33
Nodes (6): fn create_notebook, fn delete_notebook, fn list_notebooks, fn read_notebook, fn update_notebook, impl NotebookRepository for DbBackend

### Community 124 - "impl ResourceRepository for DbBackend"
Cohesion: 0.33
Nodes (6): fn create_resource, fn delete_resource, fn list_resources, fn purge_deleted_resources, fn read_resource, impl ResourceRepository for DbBackend

### Community 125 - "impl NotebookRepository for FsBackend"
Cohesion: 0.33
Nodes (6): fn create_notebook, fn delete_notebook, fn list_notebooks, fn read_notebook, fn update_notebook, impl NotebookRepository for FsBackend

### Community 126 - "impl ResourceRepository for FsBackend"
Cohesion: 0.33
Nodes (6): fn create_resource, fn delete_resource, fn list_resources, fn purge_deleted_resources, fn read_resource, impl ResourceRepository for FsBackend

### Community 127 - "impl DbBackend (server history)"
Cohesion: 0.40
Nodes (5): fn entity_history, fn server_entity_history, fn server_has_capability, fn server_http_base, impl DbBackend (server history)

### Community 128 - "impl CalendarEvent"
Cohesion: 0.50
Nodes (4): fn from_ics, fn from_ics_all, fn to_ics, impl CalendarEvent

### Community 129 - "impl PageCollector"
Cohesion: 0.50
Nodes (4): fn into_page, fn new, fn push, impl PageCollector

### Community 130 - "impl Contact"
Cohesion: 0.67
Nodes (3): fn from_vcard, fn to_vcard, impl Contact

### Community 131 - "impl HistoryRepository for DbBackend"
Cohesion: 0.67
Nodes (3): fn note_history, fn notebook_history, impl HistoryRepository for DbBackend

### Community 132 - "impl HistoryRepository for FsBackend"
Cohesion: 0.67
Nodes (3): fn note_history, fn notebook_history, impl HistoryRepository for FsBackend

### Community 133 - "impl KeeplinServer"
Cohesion: 0.67
Nodes (3): fn assemble_upload, fn from_shared, impl KeeplinServer

## Knowledge Gaps
- **1334 isolated node(s):** `EpochHeader`, `build.sh script`, `Purpose`, `Build profiles do **not** belong here`, ``[target.<triple>]` — cross-compilation (commented out)` (+1329 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `StorageError` connect `DbBackend` to `FsBackend`, `rest.rs`, `StorageError`, `CollabBackend<B>`, `StorageBackend`, `ws_sync.rs`, `LinkingBackend<B>`, `linking.rs`, `Change`, `history.rs`, `.update_notebook`, `Status`, `run_sync`, `Notebook`, ``Cargo.toml` — keeplin-daemon`, `.get_tag`, `AliasIndex`, `Note`, `.update_note`, `.get_changes_since`, `CollabHandle`?**
  _High betweenness centrality (0.110) - this node is a cross-community bridge._
- **Why does `Note` connect `Note` to `FsBackend`, `rest.rs`, `StorageError`, `run_sync`, `CollabBackend<B>`, `StorageBackend`, `DbBackend`, `.update_note`, ``Cargo.toml` — keeplin-daemon`, `search.rs`, `Result`, `note_log.rs`, `LinkingBackend<B>`, `linking.rs`, `links.rs`, `Change`, `Notebook`, `history.rs`?**
  _High betweenness centrality (0.055) - this node is a cross-community bridge._
- **Why does `DbBackend` connect `DbBackend` to `StorageBackend`, `collab_client.rs`, `ws_sync.rs`, `in_memory_backend`, `device`, `.get_tag`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
- **What connects `EpochHeader`, `build.sh script`, `Purpose` to the rest of the system?**
  _1334 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `FsBackend` be split into smaller, more focused modules?**
  _Cohesion score 0.05536435732455484 - nodes in this community are weakly interconnected._
- **Should `rest.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.060526709561574146 - nodes in this community are weakly interconnected._
- **Should `StorageError` be split into smaller, more focused modules?**
  _Cohesion score 0.09543193125282677 - nodes in this community are weakly interconnected._