# NNNN — Decision title

- Status: proposed
- Date: YYYY-MM-DD
- Decision owners: maintainer(s) who may accept or reject this ADR
- Scope: keeplin | keeplin-srv | cross-repo
- Issue: link to the originating issue
- Acceptance PR: link once the ADR is accepted
- Supersedes: none or link
- Superseded by: none or link

## Context and problem

State the verified current behavior, the problem, and why a durable decision is needed. Separate
facts, inferences, and proposals. Include evidence links that remain useful without a code diff.

## Forces and requirements

List the constraints the decision must satisfy: product needs, invariants, failure behavior,
operability, performance, compatibility, recovery, and review requirements.

## Threat model

When security or privacy is involved, identify assets, trust boundaries, adversaries,
capabilities, accepted leakage, and explicit non-goals. Otherwise state why a threat model is not
applicable; do not silently omit the section.

## Options considered

Describe each viable option fairly, including "keep the current behavior" when relevant. For
each option record benefits, costs, failure modes, operational burden, and what evidence would
change the assessment.

## Decision and justification

State one decision precisely, the invariants it establishes, and why it best satisfies the forces
above. A `proposed` ADR may state the recommendation but must not present it as approved.

## Consequences and risks

Record positive and negative consequences, residual risks, observability needs, and follow-up
work. Make non-guarantees explicit.

## Compatibility, migration, and rollback

Describe wire and format compatibility, version changes, data migration, rollout ordering across
repositories, rollback/recovery limits, and what happens to partially upgraded systems. State
"not applicable" with a reason when there is no impact.

## Verification plan

List observable acceptance evidence, including positive, negative, failure-injection,
cross-repository, migration, recovery, and operational checks proportional to the risk.

## Equivalent decision in the other repository

For a cross-repository decision, identify the canonical ADR, the companion repository link, the
immutable dependency pin/version implications, and the paired PRs/tests. For a repository-local
decision, explain why no equivalent decision is required.

