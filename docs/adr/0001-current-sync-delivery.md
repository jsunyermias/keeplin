# 0001 — Record the current synchronization delivery semantics

- Status: proposed (retrospective)
- Date: 2026-07-25
- Decision owners: maintainer
- Scope: cross-repo
- Issue: [keeplin#149](https://github.com/jsunyermias/keeplin/issues/149)
- Acceptance PR: [keeplin#164](https://github.com/jsunyermias/keeplin/pull/164)
- Supersedes: none
- Superseded by: pending the accepted design for [keeplin#150](https://github.com/jsunyermias/keeplin/issues/150)

## Context and problem

The current server relay persists accepted change batches in PostgreSQL before fan-out, deduplicates
by batch identity, and tracks a durable cursor per receiving device. However, the protocol has no
application-level acknowledgement in either direction.

On the client-to-server path, `DbBackend::send_changes` returns after the WebSocket accepts a frame,
not after `keeplin-srv` commits it. `run_sync` may then advance its timestamp watermark. A server
failure after the successful socket write but before journal commit can therefore leave the
originating client with no pending record to resend.

On the server-to-client path, the relay advances `device_cursors.last_seq` after a successful socket
send, not after the client durably applies the batch. The client drains changes into memory and
applies them one at a time; an application failure can leave the server cursor past work that was
not fully committed locally.

These windows are verified and tracked by
[keeplin#150](https://github.com/jsunyermias/keeplin/issues/150),
[keeplin#151](https://github.com/jsunyermias/keeplin/issues/151), and
[keeplin-srv#74](https://github.com/jsunyermias/keeplin-srv/issues/74). This retrospective ADR
records the actual contract so later work does not rely on the broader at-least-once claim that
appears in current prose.

## Forces and requirements

- Offline devices must receive journaled changes after reconnecting.
- Duplicate delivery must be safe through batch deduplication and idempotent `apply_change`.
- Cursor and journal retention behavior must be explicit.
- The known ingress and application acknowledgement gaps must not be hidden by an end-to-end
  at-least-once label.
- Future correction must coordinate both repositories and the shared protocol version.

## Threat model

The relevant threats are crash faults, network interruption, process restart, partial local apply,
and retry/reordering. Byzantine clients and malicious server behavior are outside this ADR. The
protected asset is user-authored change history; the primary failure is silent loss or permanent
device divergence.

## Options considered

1. Describe the complete path as at-least-once. Rejected because socket acceptance is not durable
   receiver acknowledgement, and both cursors can advance before the next durable boundary.
2. Describe the complete path as at-most-once. Rejected because journal replay and idempotent
   reapplication deliberately permit duplicates.
3. Describe each durability boundary separately and classify end-to-end delivery as unconfirmed.
   Chosen because it matches the implementation and exposes the exact recovery limits.

## Decision and justification

The current contract is recorded as follows:

- after `keeplin-srv` commits an accepted batch, the server journal is durable and server-side
  fan-out/backlog recovery prefers duplicate delivery over loss;
- batch identifiers deduplicate retries that reuse the same identifier, and entity application is
  intended to be idempotent;
- no application-level ACK proves client-to-server persistence or server-to-client durable apply;
- therefore the current end-to-end client/server sync path is **unconfirmed delivery**, not a
  verified at-least-once guarantee.

This is a description of implemented behavior, not an endorsement. The intended replacement is the
ACK/outbox design tracked by #150 and its cross-repository sub-issues.

## Consequences and risks

Operators and callers cannot infer durable remote persistence from a successful sync cycle alone.
A crash in either unacknowledged window can lose a change for another device or leave devices
divergent. Existing retry, deduplication, durable server journaling, and version-vector resolution
reduce other failure modes but do not close these windows. Documentation that calls the whole path
at-least-once must be treated as a known discrepancy until #150 replaces this ADR and updates it.

## Compatibility, migration, and rollback

This retrospective record changes no wire or data. Replacing the behavior requires a coordinated,
breaking protocol change: stable durable batch identities, acknowledgements in both directions,
bounded batches, client outbox/application transactions, cursor advancement after validated ACK,
and lockstep protocol versioning. Rollback to the unacknowledged protocol would reintroduce the
documented loss windows and must not be silent.

## Verification plan

Current evidence is the implemented sequence in `keeplin-core/src/storage/db.rs`,
`keeplin-core/src/sync/engine.rs`, and `keeplin-srv/crates/keeplin-srv/src/sync.rs`, plus the failure
analysis in #150/#151/srv#74. The replacing ADR and implementation must inject disconnections before
and after every persistence and ACK boundary, restart both sides with pending work, verify stable
deduplication, and prove that failed local application cannot advance the server cursor.

## Equivalent decision in the other repository

This file is canonical. `keeplin-srv/docs/adr/README.md` links to it rather than copying the
contract. The replacement requires paired PRs for keeplin#151 and keeplin-srv#74, an immutable
`keeplin-core` pin on the server, a lockstep protocol-version change, and cross-repository contract
tests.
