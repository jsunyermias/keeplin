# 0003 — Version persistent formats and migrate them forward explicitly

- Status: proposed (retrospective)
- Date: 2026-07-25
- Decision owners: maintainer
- Scope: cross-repo
- Issue: [keeplin#149](https://github.com/jsunyermias/keeplin/issues/149)
- Acceptance PR: [keeplin#164](https://github.com/jsunyermias/keeplin/pull/164)
- Supersedes: none
- Superseded by: none

## Context and problem

Keeplin persists user state in three physical systems: the filesystem backend, a local
LibSQL/SQLite database, and PostgreSQL in `keeplin-srv`. Format evolution must preserve existing
data, reject unsupported downgrades, survive interrupted upgrades, and keep the two repositories'
shared contracts aligned.

## Forces and requirements

- Existing stores must not be silently interpreted as a newer or incompatible shape.
- Migrations must be ordered, observable, resumable where applicable, and covered by data-preserving
  tests.
- Applied server migrations are immutable history.
- New required server columns must keep existing rows valid without an unsafe table rewrite.
- A failed upgrade needs a recovery plan that starts from a verified backup rather than pretending
  that forward schema changes can always be rolled back in place.

## Threat model

The protected asset is durable user data. Relevant failures are process crashes between migration
steps, partial deployment, accidental downgrade, schema/model drift, non-idempotent retry, and an
operator deploying a binary without a restorable backup. Malicious database administrators are
outside this ADR.

## Options considered

1. Parse whatever is on disk without a version. Rejected because incompatible layouts can be
   misread or partially overwritten.
2. Rewrite or edit old migrations as the schema evolves. Rejected because environments that already
   applied them would no longer share a reproducible history.
3. Stamp local formats and use monotonic forward migration ladders; append immutable PostgreSQL
   migrations. Chosen because it makes compatibility explicit and interrupted work recoverable.

## Decision and justification

The filesystem backend stores a format stamp in `.keeplin/format_version`, treats a missing legacy
stamp as version 1, applies every migration in order, writes the stamp after each completed step,
and refuses a store newer than the running build. The current recorded implementation is filesystem
format version 8. Verified state: every step from 2 to 8 is currently a no-op —
`apply_format_migration` performs no data transformation, so the stamp acts as a compatibility gate
rather than a data migration. This is unlike the local database ladder, whose versions 1 to 5 apply
real DDL, and the PostgreSQL migrations. A future filesystem change that alters the on-disk layout
must add a real migration step, not only bump the version.

The local database uses SQLite `PRAGMA user_version`, runs each migration in order, and refuses a
schema newer than the running build. The current recorded implementation is schema version 5.

`keeplin-srv` owns its PostgreSQL schema through ordered `migrations/NNNN_name.sql` files applied at
startup. Existing migration files are never edited after application. New migrations are
forward-only and idempotent where PostgreSQL supports it; new `NOT NULL` columns include a default
that preserves existing rows. Every SQL migration has a companion Markdown document. The current
recorded migration tip is `0016_resource_note_id.sql`.

Physical formats may differ, but shared serialized/wire fields and constants remain owned by
`keeplin-core` and follow keeplin ADR 0002.

## Consequences and risks

Upgrades are explicit and old binaries fail closed on newer local formats. The cost is permanent
migration maintenance and additional tests for every format change. Forward-only PostgreSQL
migrations mean application rollback can be constrained after a schema change; a corrective
migration or database restore may be required. A version stamp alone does not prove semantic
compatibility, so migrations and cross-repository contract tests remain mandatory.

## Compatibility, migration, and rollback

Every persistent-format change increments the relevant local format/schema version and adds a
migration step. Every PostgreSQL change appends a new numbered migration and companion; applied
files are immutable. Deployments take and verify backups before migration. Recovery uses a
corrective forward migration when possible or restores a tested backup/PITR target when not.
Cross-repository format changes use paired PRs and an immutable `keeplin-core` pin. No migration may
silently abandon user data.

## Verification plan

For local stores: test fresh stamping, every supported legacy-to-current path, data preservation,
restart after an interrupted step where the mechanism permits, and refusal of future versions. For
the filesystem backend these paths are currently trivial because no step transforms data; the first
non-trivial step must add data-preserving coverage before it lands. For PostgreSQL: test migration
from the previous schema with populated rows, repeated safe application
where intended, full `sqlx::FromRow` projections, and backup/restore drills from `RUNBOOK.md`. Run
cross-repository compatibility tests whenever serialized shared fields change.

## Equivalent decision in the other repository

This ADR is the canonical cross-repository policy. `keeplin-srv/docs/adr/README.md` links to it and
retains server-only ADRs for decisions such as a new durable queue or PostgreSQL-specific
operational dependency. Paired persistence changes must link both PRs and identify the core pin,
migration ordering, rollback/recovery boundary, and validation evidence.
