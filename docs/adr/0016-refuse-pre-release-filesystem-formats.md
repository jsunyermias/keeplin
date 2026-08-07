# 0016 — Refuse pre-release filesystem formats instead of silently relabelling them

- Status: accepted
- Date: 2026-08-07
- Decision owners: `jsunyermias`
- Scope: keeplin
- Issue: [keeplin#162](https://github.com/jsunyermias/keeplin/issues/162); unblocks
  [keeplin#207](https://github.com/jsunyermias/keeplin/issues/207)
- Acceptance PR: [keeplin#228](https://github.com/jsunyermias/keeplin/pull/228)
- Supersedes: none. Narrowly amends the `FsBackend` pre-release migration policy in
  [0003](0003-versioned-persistence.md); all other decisions in 0003 remain in force
- Superseded by: none

## Context and problem

Two requirements conflict. The issue-preparation rule used by keeplin#162 says there are to be no
data migrations or backward compatibility, while keeplin#153 requires that data never be silently
abandoned. The current filesystem startup path satisfies neither requirement honestly.

`FsBackend::FORMAT_VERSION` is 8 and its stamp is `.keeplin/format_version`. For an existing store,
`ensure_format_version` reads a missing or unparsable stamp as version 1, refuses a stamp newer
than 8, then walks every version from `current + 1` through 8. For each step it calls
`apply_format_migration`, writes that version to the stamp, and logs `Applied filesystem format
migration`. `apply_format_migration` maps every version from 2 through 8 to `Ok(())`; it transforms
nothing.

That behaviour contradicts accepted keeplin ADR 0003. ADR 0003 accurately recorded that versions
2 through 8 were no-ops and required a future layout change to add a real migration rather than
only bumping the version. Format v8 nevertheless changed attachments from the global
`resources/{uuid}/` pool to `notes/{note_id}/resources/{hash}.knrs` plus an NDJSON metadata
sidecar. `CHANGELOG.md` calls it a clean break and says the old pool is no longer read. Startup
then reports successful migrations and stamps an old store as v8 even though its old attachments
have become unreachable through `FsBackend`.

The v8 violation does not destroy the old bytes: they remain in the old pool. It silently makes
them unreachable and falsely reports that their format migration succeeded. The violation was
documented in `CHANGELOG.md` instead of being brought back through the ADR that forbade it. That
is a governance failure as well as a persistence defect. A check that paired every
`FORMAT_VERSION` bump with either a non-empty migration implementation and data-preservation test,
or an accepted ADR explicitly authorizing a refusal boundary, would have caught it. Review of the
changelog and ADR together would also have exposed the contradiction before merge.

### Verified release boundary

On 2026-08-07, `git tag -l` returned no tags and the workspace package version in `Cargo.toml` was
`0.1.0`. On the same date, the independent reviewer queried the GitHub releases API for
`jsunyermias/keeplin` and received an empty list. Acceptance must rerun that API check. The
maintainer states that no real stores exist and all stores are development data.
The repository facts are independently reproducible; the absence of real stores is an explicit
maintainer assertion, not something the tree can prove. Together they are the factual basis for a
pre-release clean break rather than a general licence to abandon user data.

### Filesystem format archaeology

The local clone is grafted at commit `67c8aa3`, where the filesystem already has format v5. The
individual v2 through v5 bump commits are therefore unavailable in this clone. The v5 source at
that boundary preserves an explicit transition inventory; the later bump commits are directly
inspectable. Where the surviving record groups versions together, this inventory does not assign
a change to one version by guesswork.

| Transition | Repository evidence | On-disk effect | Would a populated store need data transformation? |
|---|---|---|---|
| v1 → v2 | The v5 source says `LogEntry` gained `serde(alias = "note_id")` and defaults so old logs remained readable. The assignment of that change to v2 is inferred from this surviving v5 inventory; the bump commit is absent from this grafted clone. | Serialized log fields evolved while readers retained the old names and missing-field defaults. | No indication of a required transformation: the surviving source expressly describes old logs as parse-compatible. |
| v2 → v3 | The v5 source groups v3 and v4 as introducing versioned note/tag associations and resource tombstones, read through defaults on old records. It does not say which change belongs to which version, and the bump commits are absent. | One of v3/v4 changed empty association markers into versioned records; the other added resource tombstone fields. The surviving history does not identify the assignment. | No indication of a required transformation for either transition: empty or older records were deliberately interpreted through defaults. The precise v3 assignment is unknown. |
| v3 → v4 | Same surviving v3/v4 statement as above; no individual bump commit is present. | The other of the association-state and resource-tombstone changes. | No indication of a required transformation; the precise v4 assignment is unknown. |
| v4 → v5 | The v5 source says compacted global logs gained an optional `EpochHeader`, while offset cursors gained an `epoch:offset` form; headerless logs and bare offsets are interpreted as epoch 0. | New writes may carry an epoch header and two-part cursor, without invalidating the earlier forms. | No. The readers explicitly preserve the old representation as epoch 0. |
| v5 → v6 | Commit `7f0a60b` adds mandatory-at-construction `Resource.note_id`, persists it in filesystem sidecars, adds delete/restore cascades, and describes the bump as a parse-compatible no-op. The field has a serde default for old records. | Resource metadata gains note ownership; legacy records decode to the system-resource sentinel. | No byte transformation was required for parsing. Existing resources could not be assigned to a real note from information the old format did not contain, so the sentinel is the documented compatibility interpretation rather than recovered ownership. |
| v6 → v7 | Commit `4b2cfc9` replaces MessagePack note logs and sidecars with NDJSON, renames `.msgpack` paths to `.ndjson`, bumps 6 to 7, and expressly says old MessagePack is not read. | Encoding and filenames change across note logs, projections, notebook/tag/resource sidecars, and sync state. | Yes, to preserve a populated v6 store. Files would have to be decoded from MessagePack and rewritten as NDJSON at the new paths. The shipped no-op instead made old files unreachable. |
| v7 → v8 | Commit `cc95a45` and `CHANGELOG.md` move resources from the global UUID pool to per-note content-addressed blobs and metadata sidecars and expressly stop reading the old pool. | Attachment directory layout, blob naming, and metadata representation change. | Yes, to preserve a populated v7 store. Metadata must be read to determine ownership, blobs must move or be copied and content-hashed, and the new sidecars must be written. The shipped no-op instead made old attachments unreachable. |

This inventory discharges the archaeological acceptance criterion of keeplin#207, including its
unknowns. It also shows that the defect is broader than v8's attachment move: v7 was an earlier
documented clean break behind the same successful no-op ladder.

## Forces and requirements

- This decision covers only the `FsBackend` on-disk format. SQLite, `.knt`/`.kntb` containers,
  `PROTOCOL_VERSION`, and server persistence have separate evidence and decisions.
- Opening an older layout must not mutate its stamp, claim a migration succeeded, or make data
  silently disappear from the application's view.
- Failure must be explicit and actionable, naming the found and expected versions and the user's
  available recovery choices.
- Existing refusal of formats newer than the running build remains unchanged and covered.
- The old bytes must remain where they are; this decision does not authorize deletion, relocation,
  purge, or blob-atomicity work.
- The clean-break exception needs a mechanical expiry boundary. It cannot become the standing
  migration policy after users can install a published release.
- The v2 through v8 history must remain recorded now, while the evidence is relatively fresh,
  because future migrations may need that archaeology under deadline.
- keeplin#207 remains blocked while this ADR is `proposed`; only maintainer acceptance authorizes
  implementation.

## Threat model

The protected asset is filesystem-backed note and attachment data. Relevant failures are a build
silently reclassifying an old store, declaring a migration that performed no transformation,
making existing bytes unreachable, or an operator mistaking a misleading success log for proof of
preservation. Accidental downgrade remains covered by the existing newer-format refusal. Malicious
local filesystem access and the separate blob-atomicity and purge defects are non-goals.

## Options considered

### Option A — Keep the successful no-op migration ladder

This preserves automatic startup and the current tests. It also stamps layouts v1 through v7 as
v8, emits false success logs, and hides data whose paths or encodings are no longer read. It
violates ADR 0003 and is rejected.

### Option B — Implement every historical migration now

This would preserve development stores and create the migration ladder future releases will
eventually require. It has higher implementation and recovery risk, especially because parts of
the v2 through v4 provenance are unavailable in this clone and old resource ownership cannot be
reconstructed from v5 metadata. With no installable release and the maintainer's confirmation that
there are no real stores, that risk and effort buy no user-data preservation today. The archaeology
is retained so this option can be implemented when the clean-break exception expires.

### Option C — Explicitly refuse every older filesystem format until the first release

Opening a stamp below the current `FORMAT_VERSION` fails before any stamp or stored data changes.
The error identifies both versions and tells the user how to proceed. This is honest about the
pre-release clean break, preserves the bytes for manual recovery or a future migration tool, and
cannot be mistaken for successful migration. Its cost is that development stores do not open
automatically.

## Decision and justification

The maintainer accepts Option C. [keeplin#207](https://github.com/jsunyermias/keeplin/issues/207)
is unblocked for implementation.

For `FsBackend` only, opening an existing store whose parsed or implied stamp is below
`FsBackend::FORMAT_VERSION` must fail before performing any migration or writing the stamp. The
error must name the version found, or the fact that the stamp is missing or unparsable; name the
version expected; and state the honest choices: retain the untouched store for manual recovery,
start with a new store, or restore a backup already in the expected format. The exact wording may
evolve, but those three facts are invariant. This decision identifies no compatible older build
and guarantees no export or recovery tool.

The `2..=8 => Ok(())` migration arm must not survive in any form. No older version may be silently
relabelled current, and no `Applied filesystem format migration` event may be emitted when no
transformation occurred. Fresh stores may still be stamped at the current version. Stores already
at the current version continue to open. Stores newer than the build continue to follow the
existing refusal path unchanged.

This is a narrow amendment to keeplin ADR 0003. Until the expiry below, it replaces only 0003's
requirement that pre-release `FsBackend` layouts migrate forward: those older layouts are refused
instead. It does not change 0003 for SQLite, PostgreSQL, backups, downgrade refusal, shared
contracts, or any other persistent format. It does not erase 0003's data-preservation rule; after
expiry that rule governs every new filesystem format bump again.

**Mechanical expiry.** The exception expires when the canonical repository,
`jsunyermias/keeplin`, first has both (a) a git tag and (b) a non-draft GitHub release whose
`tag_name` is that tag. A published prerelease fires the boundary. The release itself, including
GitHub's automatic source archives, is sufficient; uploaded asset records are not required. The
observation is checked from `git tag -l` and the canonical repository's GitHub releases API. The
first observation creates `.github/keeplin-release-boundary.json`, recording the tag and canonical
release URL. That repository-tracked latch is immutable once committed: deletion or later removal
of the tag or release does not clear it. A tag or release that exists only in a fork is irrelevant.

Beginning with the next `FORMAT_VERSION` change after the latch is set, keeplin#153 and ADR 0003
require a real, data-preserving migration step and populated-data preservation coverage. Before
the latch is set, every later `FORMAT_VERSION` bump likewise requires either that migration and
coverage or a separately accepted ADR defining a bounded exception. The expiry is prospective,
not retroactive: it does not create migrations for v1 through v8 or make those layouts supported.

The v2 through v7 transitions remain investigated and documented even though the refusal means
they cannot execute. Once a published release exists, later format evolution will need real
migrations, and preserving the archaeology now is cheaper and safer than reconstructing it during
a release deadline.

## Consequences and risks

Opening a pre-v8 development store fails loudly without changing its stamp or bytes. Users cannot
mistake a log line or updated stamp for successful preservation. Old bytes remain available for
manual reading or a future one-off recovery tool. This ADR identifies no compatible older build
and guarantees no export path or recovery tool exists.

Development stores must be recreated or recovered deliberately. If the maintainer's assertion
that no real stores exist is wrong, affected users are blocked from opening them with the current
build. Refusal still limits harm better than silent relabelling because it preserves both the old
layout and evidence of its version.

The incomplete local provenance for v2 through v4 remains a residual risk. The inventory records
exactly what the surviving v5 source says and does not claim a transition assignment that the
available history cannot prove.

Follow-up implementation belongs to keeplin#207 only after acceptance. After the release boundary
is latched, the pre-release clean-break alternative is unavailable.

## Compatibility, migration, and rollback

This changes only opening behavior for older `FsBackend` stores. It changes no wire type,
`PROTOCOL_VERSION`, SQLite schema, `.knt`/`.kntb` container, PostgreSQL schema, or
`keeplin-core` pin. No equivalent server rollout is required.

There is deliberately no data migration for v1 through v8. An attempted open of v1 through v7 is
read-only with respect to the format stamp and stored payloads and returns an error. A v8 store is
unchanged. A future-format store remains refused by the existing path.

Rollback of the implementation would restore the dangerous successful no-op ladder, so it is not
a safe operational recovery. Recovery means retaining the untouched old store for manual reading,
restoring a backup already in the expected format, or using a future migration utility. This ADR
identifies no build capable of opening each old version and establishes no export facility.
Implementations must verify the source store before any operator-directed conversion.

## Verification plan

1. **Regression coverage:** a fresh root is stamped exactly `FORMAT_VERSION` and opens normally.
2. **Regression coverage:** a store already stamped `FORMAT_VERSION` opens without rewriting user
   data.
3. **Decision verifier:** for every old stamp from 1 through `FORMAT_VERSION - 1`, opening fails
   with an error containing the found version, expected version, and recovery choices; the stamp
   and a sentinel payload are byte-for-byte unchanged.
4. **Decision verifier:** a missing legacy stamp follows the same refusal path as implied version
   1 and no stamp is created. An unparsable stamp has a distinct explicit refusal whose error names
   the stamp as unparsable; it is tested and is not silently treated as migrated.
5. **Regression coverage:** a stamp of `FORMAT_VERSION + 1` still fails through the existing
   newer-format refusal and its current coverage remains green.
6. **Decision verifier:** no successful old-format path emits `Applied filesystem format
   migration`, and no no-op migration dispatch remains.
7. **Decision verifier:** a v7 fixture containing MessagePack files and a v8-predecessor fixture
   containing the global resource pool both fail without modifying or removing any fixture byte.
8. **Mechanical policy gate:** a repository check inspects a `FORMAT_VERSION` change and requires
   the same change to modify the migration dispatch and add a populated-data preservation test
   whose test name identifies the source and target versions, or to cite a separately accepted ADR
   authorizing a bounded exception. The check also refuses to delete or modify
   `.github/keeplin-release-boundary.json` after it first appears. This syntactic gate makes the
   required implementation and coverage mechanically visible; review still assesses whether the
   transformation and assertions are substantively sufficient. The gate lands in keeplin#207.
9. Before acceptance, rerun `git tag -l`, query the canonical repository's GitHub releases API for
   non-draft releases and their `tag_name`, confirm the workspace version, and have the maintainer
   reconfirm the absence of real stores. This check was rerun on 2026-08-07 at 20:13 UTC against
   `jsunyermias/keeplin`: `git tag -l` was empty, the GitHub releases API returned an empty list
   with no releases of any kind, the workspace version in `Cargo.toml` was `0.1.0`, and the
   maintainer reconfirmed that no real stores exist and every store is development.
10. Run `./scripts/check-docs.sh` for the ADR and registry change. Implementation checks belong to
    keeplin#207 and must not be added by this acceptance recording.

## Equivalent decision in the other repository

No equivalent `keeplin-srv` decision is required. This ADR is repository-local and changes only
`keeplin-core::storage::fs::FsBackend` startup behavior. It does not alter a shared wire or format
contract consumed by `keeplin-srv`, does not move the immutable core pin, and needs no paired
server PR or cross-repository compatibility test.
