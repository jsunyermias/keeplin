# 0002 — Keep the shared domain model in keeplin-core and materialize server projections

- Status: proposed (retrospective)
- Date: 2026-07-25
- Decision owners: maintainer
- Scope: cross-repo
- Issue: [keeplin#149](https://github.com/jsunyermias/keeplin/issues/149)
- Acceptance PR: pending
- Supersedes: none
- Superseded by: none

## Context and problem

Keeplin has two local storage backends and a multi-user server. They must agree on entity shape and
conflict semantics without maintaining independent copies of the model, while the server still
needs normalized, queryable PostgreSQL state for REST rehydration and collaboration.

## Forces and requirements

- Offline and server-backed clients must share one typed model and deterministic convergence rule.
- Local storage may use different physical layouts without changing domain semantics.
- A wiped server-mode client must be able to rehydrate from server materialized state.
- Deletions must propagate without stale peers resurrecting data.
- Server collaboration needs a line-oriented note projection, while relay payloads for other
  entities remain compatible with the core `Change` model.

## Threat model

This ADR is about consistency and data integrity rather than confidentiality. Relevant faults are
concurrent writes, reordering, duplicate delivery, stale devices, partial materialization, and
cross-repository model drift. Malicious clients and server confidentiality are handled elsewhere.

## Options considered

1. Define independent client and server domain types. Rejected because duplicated wire/model
   contracts drift and can resolve the same change differently.
2. Make the relay journal the only server state and rebuild every query from it. Rejected for the
   current system because REST rehydration and queryable collaboration need durable projections.
3. Keep canonical domain/wire types in `keeplin-core` and materialize server-owned projections in
   PostgreSQL. Chosen because it centralizes semantics while allowing storage-specific indexing.

## Decision and justification

`keeplin-core` is the single source of truth for shared entities and synchronization types. Domain
entities use UUID identity, UTC timestamps, version vectors, `last_writer`, and soft-delete
tombstones. `Change` carries mutations/snapshots across the sync boundary. Both local backends apply
the same version-vector resolution and deterministic `(timestamp, device_id)` concurrency
tiebreak.

`keeplin-srv` consumes the core types from an immutable revision and persists normalized
PostgreSQL projections. Notes in collaborative mode are represented as independently versioned
lines plus a versioned order; the body is materialized from live lines. Notebooks, tags,
note–tag associations, and resources are materialized from relay changes. The server projections
are the rehydration source of truth for server mode; the local database is a cache. The relay
journal remains delivery/history evidence, not the sole current-state query model.

## Consequences and risks

Shared types and conflict semantics cannot drift silently, but cross-repository changes require an
immutable core pin and coordinated tests. Server materialization adds a consistency boundary: a
journal commit whose projection fails can make current-state tables incomplete. That known risk is
tracked by [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) and requires its
own accepted ADR before choosing transactional materialization or a durable projection queue.

Soft deletes preserve convergence and history at the cost of retained metadata and, for resources,
payload retention until explicit age-based reclamation. Account deletion remains a separate privacy
operation that physically cascades server-owned data.

## Compatibility, migration, and rollback

Shared model or wire changes are implemented in `keeplin-core` first, pinned by immutable revision
in `keeplin-srv`, and covered by cross-repository compatibility tests. Breaking wire changes bump
the shared protocol version in lockstep. Storage-specific shape changes follow the migration rules
in ADR 0003. Rolling back one repository independently is unsafe when its pinned core contract no
longer matches.

## Verification plan

Maintain round-trip coverage for every shared protocol variant and constant, two-device
convergence tests for every versioned entity/association, stale update/delete tests, idempotent
replay tests, materialization tests for every projected entity, and cold-rehydration tests from
server state. Cross-repository CI must fail when the server pin and core contract diverge.

## Equivalent decision in the other repository

This file is canonical because the model is owned by `keeplin-core`.
`keeplin-srv/docs/adr/README.md` links here and records that server-specific projection decisions
remain local to the server registry. Server PRs that consume model changes must link the canonical
ADR, update the immutable core revision, and carry the companion compatibility evidence.
