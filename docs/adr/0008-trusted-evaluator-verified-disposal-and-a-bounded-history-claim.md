# 0008 — Trusted evaluator, verified disposal, and a bounded history claim

- Status: proposed
- Date: 2026-08-03
- Decision owners: maintainer of `jsunyermias/keeplin` and `jsunyermias/keeplin-srv`
- Scope: cross-repo
- Issue: none — successor to the 0005/0006/0007 sequence on
  [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198)
- Acceptance PR: link once the ADR is accepted
- Supersedes, once accepted: [0006](0006-trusted-review-loop-history.md) (accepted, unimplementable)
  and [0007](0007-trusted-evaluator-and-dual-store-history.md) (rejected). Until then 0006 stands.
- Superseded by: none

## Context and problem

Four designs have now tried to make the review loop's history impossible to erase. The record
is worth stating plainly, because this decision's main content is what it stops promising.

- **0004** put the history in the pull-request body. An agent can edit its own history and reset
  the stagnation brake. Reproduced.
- **0005** moved it to check-run outputs written by CI. Rejected: the writer was still code from
  the pull-request head, so it relocated the problem instead of solving it.
- **0006** added a trusted default-branch writer and a digest-chained comment journal. Accepted
  by the maintainer, then found unimplementable during implementation: a digest chain detects an
  edited or middle-deleted record because a survivor names what is missing, but deleting the
  **newest** record leaves a shorter valid chain and GitHub does not expose deleted comments.
- **0007** proposed pairing the journal with an observation count in the current head's check
  run, on the claim that check runs "cannot be deleted through the GitHub API at all". Two
  independent reviews established that this is false in effect, and one of them found that the
  ADR never specified where the count comes from — so a naive implementation recomputes it from
  the truncated journal and reproduces 0006's hole, while the test written to catch it passes.

What the two reviews jointly established about the platform:

- There is no `DELETE` endpoint for check runs, but `PATCH` rewrites a check run's output for
  any token holding `checks: write` — and a workflow's `GITHUB_TOKEN` is an installation token
  of the github-actions App, which qualifies.
- Check data is archived after 400 days and permanently removed shortly after; GitHub
  automatically deletes older same-name check runs once a suite exceeds 1,000; deleting a
  workflow run removes its job check runs.
- Repositories created before February 2023 default to a permissive `GITHUB_TOKEN` for same-repo
  pull requests, so head-controlled code may hold `checks: write` unless every pull-request
  workflow declares otherwise.

The crucial structural insight, which 0006 obscured by bundling, is that the design contained
three independent properties and let the one blocked property hold the other two hostage:

1. **The evaluator must not be editable by the pull request it judges.** Blocked by nothing.
2. **A finding's disposal must be authorized.** Blocked by nothing. Today anything may be marked
   `resolved` with no evidence at all — nothing checks the named check exists, let alone passes.
3. **The history must resist erasure, including of its newest record.** This is the hard one.

## Forces and requirements

- Deliver properties 1 and 2 now; they carry most of the value and depend on nothing unresolved.
- Every guarantee claimed must be one the implementation can demonstrate. Four designs have
  overclaimed. A smaller true statement is worth more than a larger unverifiable one.
- A test must not be able to pass while the property it names is absent — 0007's would have.
- No new operational dependency, credential store or paid service.
- Where a property is not delivered, say so in `AGENTS.md` where agents will read it, not only
  in an ADR.
- Both repositories behave identically.

## Threat model

- **Assets:** the convergence claim, the record of open findings, the stagnation brake.
- **Trust boundary:** what the pull request can influence — its body, its commits, its branch,
  and every file in its diff, which today includes the evaluator itself.
- **Adversary:** an agent inside the loop optimizing for a green check. Not a human attacker.
- **Trusted actors, enumerated rather than gestured at** — 0007 said "administrator" and that
  was too narrow. Anyone holding repository **write** access can delete the App's journal
  comments and delete workflow runs; a GitHub App or fine-grained token with `checks: write` can
  rewrite check-run output; classic tokens cannot touch check runs but can delete workflow runs.
  The loop's own agents hold such credentials. Repository-local machinery cannot defend against
  the credentials it runs on, and this decision does not pretend to.
- **What is therefore defended:** that nothing reachable *through the pull request's content* —
  its diff, its body, its commits — can edit the evaluator or forge a disposal.
- **What is not defended, explicitly:** deletion of the newest history record by an actor with
  repository write access. See Consequences.
- **Retained, as in 0004:** semantic judgment. Classifying a reifiable defect as advisory stays
  a matter for independent review.

## Options considered

**A. External append-only store.** Strongest, and the only option that actually closes property
3. Cost: a service, credentials, availability and operational burden on a pre-release project
that needs none of those today. Recorded as the escalation path.

**B. Default-branch anchor.** The evaluator commits a watermark to `main`. Durable and
protected, but continuous integration writing to the default branch on every evaluation of every
pull request is heavy churn and a broader permission than anything else here.

**C. Monotonic watermark with self-chained check runs.** The design the second reviewer proposed
to rescue 0007: the count becomes a per-pull-request watermark, each check run names its
predecessor's ID and count, and a non-empty journal with no locatable predecessor fails closed.
This is a real design and may well work. It is not chosen **now** because it rests on platform
facts neither reviewer could verify — whether deleting a workflow run removes API-created check
runs sharing its suite, whether cross-App `PATCH` is restricted, whether check runs stay listable
on dangling SHAs after a force-push. Two designs have already died on unverified platform
assumptions. Adopting a third without empirical spikes would repeat the mistake exactly.

**D. Deliver properties 1 and 2, and bound the claim on 3.** Chosen. It is the largest step that
can be taken on evidence currently in hand.

## Decision and justification

ADR 0008 supersedes 0006 and 0007.

1. **The evaluator is a default-branch `workflow_run` workflow.** Its definition comes from the
   default branch, never the pull-request head. It reads the pull request, its files, its ledger
   and the conclusions of the completed unprivileged run through APIs, and never checks out,
   executes, imports or shell-interpolates head-controlled content. Unprivileged CI keeps running
   tests read-only. This closes F-008.
2. **A reified finding reaches `resolved` or `dismissed` only against a verified *authorization*.**
   Provenance alone is not authorization — both reviews caught this independently. The reference
   is a GitHub review or comment whose author association is `MEMBER`, `OWNER` or `COLLABORATOR`
   and whose author is not the pull-request author, **and whose body carries a machine-readable
   directive naming the exact finding ID, the target state, and the reason**. An unrelated "looks
   good" from a collaborator authorizes nothing. The evaluator records the reference's immutable
   ID, author and body digest, and re-verifies all three every evaluation; a mismatched digest or
   a dismissed review returns the finding to `open`.

   `resolved` additionally names the check proving the fix, and that evidence is **bound to the
   commit being evaluated and to the configured workflow and App identity**: a success from an
   earlier head, a different workflow, or a different App does not resolve anything. Unreachable
   evidence fails closed. Tombstones and genesis records require the same verified authorization.
   This closes F-013.
3. **Every pull-request-triggered workflow declares read-only permissions explicitly**, and the
   repository's default token permission is set to read-only. This is an enforced invariant, not
   a sentence: an adversarial step in pull-request CI attempts to `PATCH` a check run and the
   build fails unless it receives `403`. Without this, head code may hold `checks: write` by
   default on a pre-2023 repository — which matters for every design in this family, including
   the ones not chosen.
4. **The journal is retained as a digest-chained sequence of App-authored comments.** Its two
   guarantees are stated separately because they are not equally strong:
   - **Editing any record is detected unconditionally.** Its successor names a digest that no
     longer matches, whether or not the tail survives.
   - **Deleting a record is detected only when a surviving descendant still commits to it.**
     Deleting a record *and its entire tail* leaves an authentic shorter prefix, which is
     terminal deletion of a longer chain and is therefore invisible. The honest phrasing is that
     deletion is caught when something downstream survives to notice it — not that "middle
     deletion is detected", which reads as unconditional and is not.

   Composing this with 5 below: the journal defends against tampering, not against truncation.
5. **Terminal deletion of history is not detected, and this is stated wherever it matters** —
   in `AGENTS.md`, in `docs/review-stalls.md`, and in the checker's own message text, which must
   state the limit rather than gesture at it: the message must say that history is verified only
   against tampering, and that an actor with repository write access can truncate it, in those
   terms. "Nothing reachable through this pull request contradicts this" is too vague to inform
   anyone. An actor with repository write access can delete the newest journal record and the
   loop will read the result as a round that has not happened yet.
6. **Only explicitly named required jobs count**, each needing positive `success` evidence.
   Skipped, neutral, absent and unknown are not green.
7. **Fork pull requests do not converge — as a deliberate fail-closed policy, not an
   impossibility.** `workflow_run` does run the default-branch definition for forks; what is
   unavailable is the journal, because the fork's unprivileged run holds no token able to append
   one. The policy is to refuse rather than evaluate on partial evidence.

Option C remains the path to property 3, and is **not** filed as vague future work: without a
tracking artifact a "documented path" is limbo, and limbo is surrender by starvation. Accepting
this ADR therefore also opens a follow-up issue for a three-probe spike against a scratch
repository, settling exactly the facts that killed 0007: whether deleting a workflow run removes
API-created check runs sharing its suite; whether cross-App `PATCH` of a check run is restricted;
and whether check runs stay listable on dangling SHAs after a force-push. The spike is
specifiable today. Its result decides whether option C proceeds to a superseding ADR — not an
amendment to this one — or whether option A becomes necessary.

## Consequences and risks

Positive: the evaluator stops being editable by what it judges; disposal stops being an
assertion, which was the largest real hole in the whole mechanism; the permissions invariant
closes an exposure that existed independently of any of this; and the remaining limitation is
written where agents read it rather than buried.

Negative and residual:

- **The stagnation brake is defeatable by an actor with repository write access** who deletes the
  newest journal record. The brake still works against an agent that merely edits the pull-request
  body, which is the common case, but the strong claim is withdrawn.
- `AGENTS.md` and ADR 0004's language must be corrected, not merely supplemented: 0004 says
  indefinite silent iteration "ceases to be reachable", which was never true.
- Elevated permissions on the trusted job, never exposed to a checkout, shell, head dependency
  or any action other than GitHub's own, pinned to a full commit SHA.
- Two properties delivered out of three is a partial result, and calling it complete would repeat
  the error this decision exists to correct.

## Compatibility, migration, and rollback

No wire, protocol, format or persistence surface is touched; the change is confined to
`.github/` and `docs/`. Open pull requests have no journal; theirs begins at the first evaluation
after this lands, recorded as a genesis observation carrying a verified maintainer reference.
Rollback is reverting the pull requests; journal comments and check runs left behind are inert.

## Verification plan

Deterministic tests, each failing before the change, and each written so it cannot pass while the
property is absent — the specific defect in 0007's plan:

- **Trusted evaluator:** a record whose App, workflow, repository or schema does not match
  configuration is refused; the trusted workflow contains no checkout of head content and no
  interpolation of head-controlled values, asserted structurally against the workflow file.
- **Permissions invariant:** a step in pull-request CI attempting `PATCH /check-runs/{id}`
  receives `403`; the build fails if it succeeds.
- **Verified disposal:** a disposal whose reference is missing, unauthorized, authored by the
  pull-request author, digest-mismatched, or a dismissed review leaves the finding `open`.
- **Journal, for what it does cover:** an edited record and a middle-deleted record both yield
  `history-unverifiable`.
- **Journal, for what it does not:** `limitation_F002_terminal_truncation_undetected`, asserting
  the concrete behaviour — a truncated journal evaluates as if the missing rounds never happened.
  It must share its fixture with the positive tests above. Asserting mere absence of detection
  would pass even if the checker stopped reading the journal entirely, letting a total breakage
  masquerade as a documented limitation. The name marks it as a limitation pin, and the ADR that
  delivers property 3 is required to delete it.
- **Required checks:** skipped, neutral, absent and unknown do not converge.
- ADR 0004's inherited semantics continue to pass unchanged.

The deliberately-red F-002 test is retired rather than made green: it reifies a property this
decision does not deliver, and a permanently red test for a formally undelivered property is a
specification error, not a failing test. Both reviews agreed that is honest accounting rather
than the reclassification `0.B` forbids — the difference being that the reclassification here
*is* the reviewed decision.

It does **not** get a new ledger state. `AGENTS.md` defines exactly four — `open`, `resolved`,
`dismissed`, `advisory` — and inventing a fifth to describe an inconvenient case is precisely the
move this sequence exists to prevent. F-002 therefore stays `open` until this ADR is accepted,
and becomes `dismissed` at acceptance, citing this decision and linking the spike below. The
docs check is extended to reject any ledger state outside the four.

## Equivalent decision in the other repository

Canonical here in `jsunyermias/keeplin`; `jsunyermias/keeplin-srv` links it and carries the
byte-identical evaluator, suite and trusted workflow, differing only where its CI genuinely
differs. No `keeplin-core` surface or pin is affected.
