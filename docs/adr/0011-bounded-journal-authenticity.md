# 0011 — Bound the review journal's authenticity claim

- Status: accepted
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — round-12 review findings on
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
- Acceptance PR: [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198), with
  [keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104) as its companion
- Supersedes: [0008](0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md)
  on its journal-authenticity claim only
- Superseded by: none

## Context and problem

ADR 0008 correctly bounds terminal truncation, but overstates what authenticates a surviving
journal record. The evaluator accepts a record when its issue comment reports the configured
`github-actions` App identity and the record carries a matching SHA-256 digest of its own fields.
That digest is unkeyed.

Two mechanical reproductions establish distinct failures of the stronger claim:

1. Every repository workflow receives a `GITHUB_TOKEN` issued for the same `github-actions` App.
   A workflow added or modified in this repository can request `issues: write`, write or replace
   comments with that App identity, and recompute the unkeyed digest. In particular, `ci.yml`
   runs on pushes to `claude/**` using the workflow definition from that branch.
2. A comment carrying the journal marker whose JSON does not parse is currently skipped. When
   the corrupted comment is the newest record that established reification, the evaluator reads
   one record instead of two and can converge with the finding classified as advisory.

The second defect is an implementation bug and is fixed under ADR 0008's existing fail-closed
invariant. The first is a threat-model boundary: the journal has no secret or independently
authenticated writer that distinguishes the evaluator from another repository workflow.

This ADR supersedes ADR 0008 only for the following authenticity statements:

- Decision item 4: “Editing any record is detected unconditionally.” and its explanation that a
  successor or the record's own digest necessarily exposes the edit.
- Decision item 4's conclusion: “the journal defends against tampering, not against truncation.”
- Decision item 5's required message claim that “history is verified only against tampering”.
- The verification-plan statement that “an edited record” necessarily yields
  `history-unverifiable`.

Those sentences are replaced by the bounded claim below. Every other decision in ADR 0008
continues to stand, including default-branch evaluation, verified finding disposal, explicit
permissions, required-job semantics, fork refusal, and the terminal-truncation bound.

## Forces and requirements

- State no authenticity property that the App identity plus an unkeyed digest cannot establish.
- Preserve the useful detection of accidental corruption and casual edits through the pull
  request without presenting it as resistance to a capable repository workflow.
- Preserve ADR 0008's terminal-truncation sentence verbatim and keep its limitation tests.
- Fail closed when an App-authored comment carries the journal marker but its payload cannot be
  parsed or validated.
- Keep both repositories' evaluator, workflow, policy surfaces, tests, and companions
  byte-identical.
- Add no credential, service, environment, wire-format version, or persistence dependency under
  this bounded decision.
- Leave acceptance and any stronger provenance design to the maintainer.

## Threat model

The protected assets are the surviving review-history sequence, the remembered classification
of finding IDs, the stagnation brake, and the convergence result derived from that history.

If accepted, the journal detects accidental corruption and casual editing by a person or agent
working through the pull request: changing a recorded byte without also rebuilding the journal
digest and chain fails verification, and malformed App-authored marked payloads fail closed.

The journal does **not** defend against an actor able to add or modify a workflow in this
repository. Such a workflow can obtain a `GITHUB_TOKEN` carrying the same `github-actions` App
identity, request `issues: write`, and recompute the unkeyed record digest. App identity therefore
identifies the shared GitHub Actions installation, not the trusted evaluator workflow.

A determined actor with that access can manufacture a valid-looking history in which a finding
was never reified. The evaluator can then converge on the manufactured history. This remains
possible even when every surviving record and chain link is internally consistent.

The decision does not attempt to defend against repository administrators, compromised GitHub
infrastructure, stolen credentials, or ADR 0008's already documented terminal truncation. It
also does not replace independent review, branch protection, or semantic judgment about whether
a finding is reifiable.

## Options considered

### Bound the threat model

Retain the App-authored, digest-chained journal as an integrity check against mistakes and casual
pull-request editing, fail closed on every marked payload the configured App authored, and state
plainly that repository workflows can forge it. This adds no operational secret and matches the
evidence, but determined workflow-level forgery remains possible.

### Authenticate records with an environment-held HMAC key

Compute an HMAC over every record with a secret held in a GitHub Environment whose deployment-
branch policy admits only the default branch. A `claude/**` workflow could not read that secret,
so it could not manufacture a valid record merely by sharing the `github-actions` App identity.
This would provide cryptographic separation, but it introduces secret generation, rotation,
availability, incident recovery, environment policy configuration, and a migration story for
existing unsigned records. A failure or policy drift could also stop every evaluation.

The maintainer chose the bounded threat model rather than accept that new credential and
operational boundary for this pre-release repository. Evidence that workflow forgery becomes a
practical rather than residual risk, or a future requirement for cryptographic provenance, would
justify a new ADR reconsidering the HMAC option.

### Keep ADR 0008's unconditional wording

No code or operational change, but mechanically false. Rejected because a determined workflow
can satisfy both inputs the evaluator currently treats as authenticity evidence.

## Decision and justification

If accepted, replace ADR 0008's journal-authenticity claim with this statement:

> The App-authored digest chain detects accidental corruption and casual record editing through
> the pull request when the editor does not also reconstruct the unkeyed chain. It does not
> authenticate records against another repository workflow, because that workflow can use the
> same App identity and recompute every digest.

The evaluator must fail closed on a comment that carries the journal marker, is attributed to
the configured App identity, and does not contain a complete parseable journal record. Finding
IDs and recorded authorization evidence continue to be interpreted only from surviving valid
records, subject to ADR 0008's unchanged truncation bound.

This is the smallest statement supported by the current mechanism and the maintainer's chosen
operational posture. Because this ADR is `proposed`, it authorizes no HMAC, secret, Environment,
or other provenance implementation.

## Consequences and risks

Policy and runtime messages stop presenting an unkeyed digest as cryptographic provenance. The
existing chain still catches common accidental edits, malformed marked payloads fail closed, and
the surviving-descendant deletion check remains useful within the bounded threat model.

Residual risk is explicit: a workflow-capable actor can forge or rewrite the complete visible
history and manufacture convergence, including a history in which no finding was ever reified.
Independent review and protected-branch governance remain necessary controls outside the
journal's claim.

Operational observability remains the evaluator's `history-unverifiable` result for malformed,
digest-mismatched, misordered, or identity-mismatched surviving records. There is no reliable
local signal for a well-formed forgery made with the shared App identity.

Follow-up work is required only if the maintainer later chooses authenticated provenance; that
work needs a new accepted ADR covering HMAC key lifecycle and migration.

## Compatibility, migration, and rollback

No Rust API, wire protocol, persistent format, database schema, or `keeplin-core` dependency pin
changes. Both repositories receive the same policy wording and evaluator bug fix in coordinated
commits; partially updated repositories differ only in their claims and test coverage, not in
client/server behavior.

There is no journal-format migration. Existing records remain readable. Malformed marked
comments that were formerly ignored instead make history unverifiable, which is the intended
fail-closed correction. Rollback is a coordinated revert of the policy and evaluator commits;
journal comments created in the interim remain ordinary v1 records. Rolling back would restore
the known silent-skip defect and the overclaim, so it is recovery from an operational regression,
not a security improvement.

## Verification plan

- A configured-App comment carrying the journal marker and malformed JSON returns
  `history-unverifiable`; the same test must fail against the pre-fix evaluator.
- A digest mismatch, producer mismatch, malformed payload shape, and broken predecessor link
  continue to fail closed.
- The intact journal fixture and the canonical terminal-truncation limitation test continue to
  pass, showing that the fail-closed parser did not silently disable journal reading or enlarge
  the claim.
- Repository policy, runtime messages, workflow companions, and templates contain the bounded
  accidental/casual-editing claim and no live unconditional tamper-resistance claim.
- `./scripts/check-docs.sh`, the Node evaluator suite, and the Python documentation-check suite
  pass in both repositories.
- `cmp` confirms that every shared governance file is byte-identical between repositories.

The absence of an HMAC cannot be proven as a security property. Verification therefore includes
the explicit negative claim and review of workflow token capabilities, rather than a test that
pretends a capable workflow cannot forge the journal.

## Equivalent decision in the other repository

This file in `jsunyermias/keeplin` is canonical. The `jsunyermias/keeplin-srv` ADR registry links
keeplin ADR 0011 and mirrors its status and server impact. Both repositories carry byte-identical
governance files, evaluator implementation, workflow, companions, and tests. No immutable
`keeplin-core` pin or cross-repository protocol version changes; the coordinated pull requests
are [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) and
[keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104).
