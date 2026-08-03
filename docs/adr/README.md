# Architecture decision records

This directory is Keeplin's durable record of high-risk architectural decisions. An ADR
separates a maintainer-approved decision from the implementation proposed by an author or
agent. A reviewer must be able to evaluate the decision from the ADR and its evidence without
reading a code diff.

Cross-repository decisions live here, in `jsunyermias/keeplin`, because `keeplin-core` owns the
shared model, wire, and format contracts. A decision specific to `keeplin-srv` lives in that
repository's `docs/adr/` registry. Each side links the other instead of copying a decision.

## When an ADR is required

An accepted ADR is required before implementation when a change does any of the following:

- changes a protocol or cross-repository compatibility contract;
- changes persistence, migration, retention, or creates a risk of data loss;
- changes cryptography, authentication, authorization, permissions, or privacy;
- changes synchronization, delivery, conflict resolution, or recovery guarantees;
- adds an operational dependency or changes a production failure boundary;
- removes or weakens an existing protection.

An ADR is normally not required for an implementation that follows an already accepted ADR,
a mechanical refactor that preserves every contract, a documentation correction that makes no
new decision, or a bug fix that restores an already documented invariant. When the boundary is
unclear, open a proposed ADR and resolve that question before production implementation.

### Relocation is not a decision

Splitting a file or moving code between modules keeps reaching this boundary, because the code
it moves — persistence, migrations, protocol — is named in the triggers above. It is not a
decision and needs no ADR when the change preserves, and the pull request demonstrates that it
preserves, all four of:

- every public path a caller outside the moved tree can name;
- observable behavior, including error paths and ordering;
- every persisted or on-wire format, and the version constants that gate them;
- the migration sequence, including which migration runs at which schema version.

Show the four with the evidence that establishes them, not as an assertion. A relocation that
cannot show all four is not a relocation, and the boundary applies normally.

Maintainer decision of 2026-07-27, from the question raised in
[keeplin#178](https://github.com/jsunyermias/keeplin/pull/178). It is deliberately narrow: the
defects relocation actually produces are stale references and inherited container inventories,
which no ADR would have caught. [keeplin#179](https://github.com/jsunyermias/keeplin/issues/179)
is what covers those, and it is scheduled before the remaining fragmentation for that reason.

## Lifecycle

The allowed states are:

- `proposed`: under review; implementation that depends on the decision is blocked;
- `accepted`: approved by the maintainer and safe to implement;
- `rejected`: considered and explicitly not chosen;
- `superseded`: replaced by a later accepted ADR.

Only a maintainer may move a decision to `accepted` or `rejected`. Once accepted, the decision
body is immutable: do not rewrite history to match a later implementation. A later decision gets
a new number and marks the old ADR `superseded`. The only edits allowed to an accepted ADR are
status/cross-link metadata that point to its replacement and corrections that do not alter the
decision; substantive corrections require a new ADR.

Retrospective ADRs record verified behavior already present on `main`. `(retrospective)` is a
qualifier that may accompany any status, so `proposed (retrospective)` and `accepted
(retrospective)` are both valid: the status tracks maintainer approval, while the qualifier records
that the ADR documents implemented behavior rather than proposing new behavior. `accepted
(retrospective)` means "this is the implemented state", not "this state is ideal". Known defects
remain defects and are linked explicitly; fixing one requires the normal issue/ADR process when
the fix crosses a decision boundary.

## Numbering and links

- Copy [`0000-template.md`](0000-template.md) to the next unused four-digit number.
- Use `NNNN-short-kebab-title.md`. Numbers are permanent and never reused, including after a
  rejection.
- Numbering is per repository, so a bare number is ambiguous across the two registries. Always
  qualify a cross-file reference with its repository: `keeplin ADR 0002`, `keeplin-srv ADR 0001`.
- Every ADR links its originating issue and the PR that accepts it. Issues and PR descriptions
  link back to the ADR.
- A superseding ADR links the superseded ADR; the old ADR receives only the reciprocal metadata
  link.
- Cross-repository ADRs identify the canonical file and the equivalent record/link in the other
  repository. Server-only ADRs remain in `keeplin-srv`.

## Author and reviewer workflow

1. Confirm the issue has observable criteria and identify the decision boundary.
2. Copy the template, fill every applicable section, and leave the status `proposed`.
3. Present evidence, alternatives, failure modes, compatibility, migration, recovery, and
   rollback independently of any implementation diff.
4. Obtain maintainer approval and change the status to `accepted` in the ADR PR.
5. Only then implement dependent production changes in a dedicated PR, linking the accepted ADR.
6. Use a new ADR to replace an accepted decision; never edit its reasoning retroactively.

## Registry

| ADR | Status | Scope | Related work |
|---|---|---|---|
| [0001 — Current synchronization delivery semantics](0001-current-sync-delivery.md) | accepted (retrospective) | cross-repo | [keeplin#150](https://github.com/jsunyermias/keeplin/issues/150), [keeplin#151](https://github.com/jsunyermias/keeplin/issues/151), [keeplin-srv#74](https://github.com/jsunyermias/keeplin-srv/issues/74), [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) |
| [0002 — Shared domain model and server projections](0002-shared-domain-model.md) | accepted (retrospective) | cross-repo | current implementation |
| [0003 — Versioned persistent formats and forward migrations](0003-versioned-persistence.md) | accepted (retrospective) | cross-repo | current implementation |
| [0010 — E2EE collaborative editing: threat model and v1 ambition](0010-e2ee-collab-threat-model.md) | proposed | cross-repo | [keeplin#143](https://github.com/jsunyermias/keeplin/issues/143), [keeplin#142](https://github.com/jsunyermias/keeplin/issues/142), [keeplin#154](https://github.com/jsunyermias/keeplin/issues/154), [keeplin-srv#72](https://github.com/jsunyermias/keeplin-srv/issues/72) |

The first prospective use of this framework is
[keeplin#143](https://github.com/jsunyermias/keeplin/issues/143). Its ADR now exists as 0010 and
is `proposed`: it still needs maintainer acceptance before cryptographic or collaboration
implementation begins, and it carries four decisions marked for the maintainer, including the one
[keeplin#162](https://github.com/jsunyermias/keeplin/issues/162) assigned to it — whether granular
encryption survives a blind server.
