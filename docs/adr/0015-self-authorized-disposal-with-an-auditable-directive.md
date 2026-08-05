# 0015 — Self-authorized disposal with an auditable directive

- Status: accepted
- Date: 2026-08-05
- Decision owners: `jsunyermias`
- Scope: cross-repo
- Issue: [keeplin#206](https://github.com/jsunyermias/keeplin/issues/206)
- Acceptance PR: [keeplin#217](https://github.com/jsunyermias/keeplin/pull/217)
- Supersedes: none. Amends the authorization precondition that
  [0008](0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) established and
  [0013](0013-genesis-anchor-on-an-empty-journal.md) applied to genesis
- Superseded by: none

## Context and problem

### The verified defect

`.github/scripts/check-review-loop.js` disposes of a finding only through `verifyAuthorization`,
which requires a directive whose author simultaneously satisfies two conditions:

- the author is **not** the pull request author;
- the author's `author_association` is `MEMBER`, `OWNER` or `COLLABORATOR`.

A collaborator query run on 2026-08-05 against both repositories returned exactly one principal:

| Repository | Principal | Role |
|---|---|---|
| `jsunyermias/keeplin` | `jsunyermias` | admin |
| `jsunyermias/keeplin-srv` | `jsunyermias` | admin |

No app, bot or service account holds a sufficient association. The same principal opens the pull
requests. The two conditions therefore cannot be satisfied simultaneously by anyone. This resolves
acceptance criterion 0 of keeplin#206, which the issue left open: the path is *unreachable*, not
merely undocumented.

### The consequence is wider than keeplin#206 states

keeplin#206 describes the defect as blocking convergence through the synthetic `GENESIS` finding.
Measurement on a real pull request shows it is broader.
[keeplin-srv#114](https://github.com/jsunyermias/keeplin-srv/pull/114) was the first pull request
the default-branch evaluator ever evaluated. Its published journal records report:

```json
{"observation": 3, "unauthenticatedAnchor": true, "blocking": 15}
```

Fifteen blockers: `GENESIS` plus **fourteen ledger findings that were fixed and independently
verified**, each carrying the same diagnosis:

```json
"disposalError": "authorization reference is unreachable"
```

The mechanism is in `evaluateTrustedReviewLoop`: any finding whose ledger state is `resolved` or
`dismissed` and whose authorization does not verify is projected back to `reified: true,
state: "open"`. That behaviour is correct — the evaluator must not accept prose as evidence of
disposal — but combined with the unreachable authorizer it means **no finding in either
repository can ever be recorded as closed**, whether or not its pull request would otherwise
converge.

That is a stronger statement than keeplin#206 makes, and it is the fact this decision must
answer.

### Evidence of the practical outcome

Three pull requests have now merged above an open stall record rather than through the gate:
keeplin#203, cited by keeplin#206 itself; keeplin-srv#114; and **keeplin#215, the pull request that
proposed this ADR** — which could not open the gate it documents and so went the same way,
making the argument below about itself. Of these, keeplin-srv#114 is the one whose
`docs/review-stalls.md` row remains open on `main` with all fifteen blockers named and
`Exit taken` empty. `AGENTS.md` defines an open row there as an open condition under *Definition
of done*. The gate that eleven ADRs built is being routed around because it cannot be opened.

### Facts, inferences and proposals

- **Fact**: the collaborator query result above, the evaluator's published `blocking: 15`, and the
  fourteen `authorization reference is unreachable` diagnoses.
- **Fact**: `verifyAuthorization` binds repository, pull-request number, non-author authorizer,
  exact finding, target state, and recorded reference identity, author and body digest. It does
  **not** bind a head SHA or a nonce; that limitation is recorded in
  [0014](0014-correct-the-empty-journal-decision-record.md).
- **Inference**: no configuration of the current repositories admits a qualifying authorizer
  without adding a principal.
- **Proposal**: everything under *Decision and justification*.

## Forces and requirements

- **The gate must become openable.** A convergence condition no participant can satisfy is not a
  high standard; it is a broken mechanism that trains its users to merge around it.
- **Disposal must remain deliberate and attributable.** Whatever replaces the separation must
  leave a record that a later reader can audit without the transcript of an agent session.
- **The bounds of 0008 and 0011 must not be overstated.** The journal's chain is unkeyed: it
  detects accidental corruption and casual editing, not forgery by a repository workflow carrying
  the same App identity, and terminal truncation is not detected. No option may be presented as
  closing those holes.
- **Reification must remain irreversible by shortcut.** A finding that named a mechanical check
  may not become `advisory` by relabelling.
- **The relaxation must be detectable if the project grows.** A rule justified by "one human" must
  fail loudly, not silently persist, once that premise stops holding.
- **Independent review is untouched.** `AGENTS.md` states that convergence is never a substitute
  for an independent reviewer, and that an agent does not waive that review. This decision changes
  what *convergence* requires; it changes nothing about who may review.

## Threat model

**Asset.** The claim, carried in the journal and the ledger, that a reified finding was disposed
of deliberately by someone entitled to decide.

**Where the evidence lives, and who can write it.** Stated explicitly because the rest of this
section cannot be evaluated without it, and because an earlier draft omitted it.

| Artifact | Location | Writer |
|---|---|---|
| Review ledger | the pull request **body** | anyone who can edit the pull request |
| Journal records | pull request **comments** | only the configured App: `verifyJournal` refuses any comment whose `app_slug`/`app_id` do not match `config` |
| Authorization directive | the **body of a review or comment** it is carried in | its author, whose `author_association` is checked |

**None of the three is in the file diff.** All three are visible on the pull request page. A
reviewer who reads the pull request sees them; a reviewer who reads only `git diff` does not.

**Trust boundary.** Between anyone who can write to the repository or its workflows, and the
evaluator running from the default branch.

**Adversaries and capabilities.**

1. *An agent or contributor wanting a pull request to converge.* Can edit the ledger and pull
   request body, and open comments. Cannot, today, produce a verifying directive.
2. *An actor with repository write access.* Can add or modify a workflow that runs under the same
   App identity, recompute the unkeyed digest chain, and manufacture convergence on a history in
   which no finding was ever reified. Can also truncate the newest journal record.

Recomputing the chain is not merely a matter of write access to a file: journal records live in
comments and `verifyJournal` refuses any comment not attributed to the configured App, so
adversary 1 cannot rewrite history without reaching that identity. The chain's weakness is that
the App identity is shared with any repository workflow, not that the records are loosely
writable.

**What the current separation actually buys against each.** Against adversary 1 it is real: an
agent cannot self-dispose. Against adversary 2 it buys nothing, because that actor controls the
identity the chain is built on. 0008 and 0011 already concede this.

**Accepted leakage and non-goals.** This ADR does not authenticate history against adversary 2,
does not detect terminal truncation, and does not introduce a keyed chain. Those remain the bounds
of 0008 and 0011 and are unchanged.

**The honest statement of what is being given up.** In a repository whose only principal is the
maintainer, the author/authorizer separation defends against *the maintainer's own agents acting
without the maintainer*, not against a second human. Options A and C below both remove it — A
unconditionally, C while the single-principal premise holds — and replace it with an explicit,
attributable act by the maintainer. That is a real reduction in defence against adversary 1, and
it is the cost either of them accepts.

## Options considered

### Option A — Self-authorization with an auditable directive

The pull request author may authorize disposal of a finding on their own pull request, provided
the directive is recorded as a first-class, machine-readable artifact bound to that pull request
and finding.

- **Benefits.** Opens the gate with **no new principal**, and with no credential *on the
  authorization path*. Keeps every disposal an explicit, dated, attributable act rather than an
  implicit consequence of editing a table. Preserves the evaluator's refusal to accept prose.
- **Costs.** Removes the author/authorizer separation. Against adversary 1 this is a genuine
  reduction: an agent operating with the maintainer's credentials could issue the directive. The
  mitigation is attribution and reviewability, not prevention.
- **A cost earlier drafts of this ADR omitted.** They claimed Option A needs "no new credential".
  That claim was never verified and is withdrawn. Whether the evaluator's existing token can list
  collaborators is genuinely unsettled — *Decision* records both readings of the documentation and
  makes one real call a precondition of acceptance. **Option C carries exactly the same
  requirement**, since it performs the same enumeration; the credential is not a difference
  between them.
- **Failure modes.** A maintainer who authorizes without reading disposes of findings by habit.
  The directive's audit trail makes that visible after the fact; nothing prevents it.
- **What would change the assessment.** A second human principal joining the project, which would
  make the separation meaningful again and this relaxation unnecessary.

### Option B — A service principal issues the directives

An app or bot account with sufficient association authorizes disposals.

- **Benefits.** Preserves the separation formally, and — this is a real advantage an earlier draft
  of this ADR collapsed away — creates a **capability boundary**. An agent operating with the
  maintainer's ordinary credentials could not authorize without also reaching the separate
  credential. Under Option A it can. A credential held behind an Environment approval, with
  least privilege and its own revocation and audit trail, is materially different from Option A,
  not "Option A with a credential in between".
- **Costs.** Operational burden: a credential to hold, rotate and protect, plus an approval step
  on a path that runs many times per pull request. Because *every* disposal needs authorization,
  not only genesis, that step recurs per finding.
- **Failure modes.** If no human judgement sits behind the step, it becomes a rubber stamp that
  *looks* like independent authorization and is therefore worse than Option A, which is at least
  honest about what it is. Whether that is the likely outcome here is a **design judgement, not a
  measured claim**: this ADR asserts it because the same person would drive both sides, and offers
  no evidence beyond that.
- **On compromise.** An earlier draft said compromising the credential "converges any pull
  request". That is overstated. Convergence still requires the required jobs green and, for
  `resolved`, a success check bound to the head, workflow, run and App. What compromise actually
  buys is arbitrary authorization of disposals, which manufactures convergence *when the other
  conditions already hold*.

### Option C — Relaxed association for single-principal repositories

Declare an explicit policy exception when the repository has exactly one principal with a
sufficient association.

- **Benefits.** Same effect as Option A, framed as a bounded exception with a detectable
  precondition.
- **Costs.** The precondition must be evaluated at authorization time against live repository
  state. That is **not a new API dependency** — an earlier draft claimed it was, wrongly: the
  evaluator already reads `author_association` from the API on this very path. It is a further
  call of the same class, with its own failure mode and the policy complexity of deciding what
  happens when it fails. **The enumeration carries the same credential requirement as Option A's
  guard** — same endpoint, same unsettled question recorded under *Decision*. The surviving
  distinction is placement, not cost.
- **How it really compares to Option A — and the comparison moved twice.** An early draft said
  Option A "reaches the same outcome without it". It does not: the premise check gives
  Option A a live collaborator lookup too. A later draft then argued the surviving distinction —
  **where the lookup sits** — settled it for A, because C's failure could wrongly permit an
  individual disposal. **That argument is also withdrawn.** *Wrongly permitted* is not inherent to
  placing the lookup on the authorization path; it is a consequence of designing C fail-open, and
  the `unknown ⇒ refuse` rule this ADR already decides removes it at no extra machinery. What
  remains is one refused disposal, retryable, against a blocked gate. **The smaller radius is C's.**
- **Where C is genuinely better, and it is not nothing.** The two options behave differently at
  the moment that matters most — when the premise decays. Under C the precondition is evaluated at
  authorization time, so a second principal makes the exception **lapse on its own** and the
  repository falls back to the original author/authorizer separation, which works again precisely
  because a second qualifying principal now exists. Under A the guard **blocks the gate** until
  someone intervenes. Lapsing safely and failing loudly are different outcomes, and this one
  favours C. An earlier draft wrote "for the same result", which papered over it.
- **Assessment.** A genuine peer of Option A, not a costlier route to the same place. On the two
  failure events this ADR treats as decisive — a transient lookup failure, and decay of the
  single-principal premise — C is equal or better. Its cost is a live lookup on the authorization
  path and a per-disposal failure policy to design. *Decision* records why this ADR therefore
  stops short of recommending between them.

### Option D — Pre-seeded genesis anchor

Seed the journal on the default branch so it is never empty.

- **Assessment.** **Insufficient, and the measurement shows why.** It addresses `GENESIS` and
  leaves every ordinary finding undisposable. The durable form of the argument does not depend on
  the exact count: *pre-seeding removes at most the synthetic blocker and leaves untouched every
  ordinary blocker whose authorization does not verify.* The specific `blocking: 15` → 14 reading
  comes from keeplin-srv#114's journal and is **an external result this ADR has not had
  independently re-derived**; a reviewer should confirm it from the linked journal rather than
  from this document. Recorded so the option is not revisited without that evidence.

### Option E — Keep the current behaviour

- **Assessment.** The status quo is that pull requests merge above open stall records, and
  `AGENTS.md` declares the default branch not-done while such a record is open. Three merges have
  now taken that route, the most recent being this ADR's own pull request. Keeping it means the convergence rule exists only on paper.

## Decision and justification

> `proposed`. The maintainer indicated Option A as the intended direction; that is a statement of
> intent recorded here, not an approval. Implementation remains blocked until this ADR is
> `accepted`.
>
> **Accepted with Option C.** After the withdrawal below, the maintainer chose **Option C**: relax
> the author condition only while the single-principal premise holds, checking it at authorization
> time with `unknown ⇒ refuse`. Everything this ADR labels *the Option A form* — the locus and
> cadence bullets, and the A column of verification item 9 — is therefore the **rejected**
> alternative, kept as the record of what was considered and why it lost, not as a live option.
>
> **The maintainer subsequently approved withdrawing the `recommended` label from Option A**, after
> the two independent reviewers split on whether it still followed from the comparison below. That
> approval is recorded here rather than left traceable only to a review transcript: without it, a
> later reader would find a recommendation removed from the option the maintainer had asked for,
> with nothing in the record saying who decided that.

**As proposed, this ADR decided a mechanism and left the policy choice between Options A and C to
the maintainer; on acceptance the maintainer chose Option C.** The `recommended` label an earlier
draft carried on Option A had been withdrawn before that choice, and the reason is recorded rather
than quietly dropped — the withdrawal is why the choice was the maintainer's to make rather than a
conclusion this document reached on its own.

Review removed, one at a time, most of what distinguished A from C. A was said to need
no live lookup — it does. A was said to need no credential — it does. A was said to reach the same
detectability more cheaply — it does not; the surviving difference is where the lookup sits. When
the last of those fell, an independent reviewer showed that the remaining difference does not
favour A either: against an Option C specified with the same `unknown ⇒ refuse` rule this ADR
already decides for the enumeration, C's failure refuses **one disposal**, which is retryable,
while A's failure
blocks **the whole gate**; and when the premise decays, C lapses back into the original separation
on its own — which works again precisely because a second qualifying principal now exists — where
A blocks until someone intervenes. In both events this ADR treats as decisive, C is equal or
better.

The other reviewer read the same text and held that the recommendation survived, thinned to its
real argument. **They disagreed, and this ADR does not paper over it.** What both readings agree
on is narrower and is what gets decided here: the *mechanism* — the directive, its conditions, and
the guard on the single-principal premise — is common to A and C. Only the placement of the
premise check differs, and that placement is a policy trade the maintainer owns:

- **Option A** relaxes the author condition unconditionally and checks the premise off the
  authorization path. Simpler on that path; larger blast radius on failure; requires deliberate
  action when the premise decays.
- **Option C** relaxes it only while the premise holds, checking at authorization time with
  `unknown ⇒ refuse`. Smaller blast radius; self-lapsing on decay; adds a live lookup to the
  authorization path and a per-disposal failure policy to design.

Choosing A remains defensible. What is no longer defensible is presenting A as the *result* of the
comparison written above. A maintainer who still prefers A should record that preference as a
preference; the acceptance pull request implements whichever is chosen, and everything below
applies to both unless it says otherwise.

**The mechanism.** The pull request author may authorize disposal of a finding on their own
pull request when the authorization is carried by a directive satisfying **all** of the following.
This list is deliberately **not** offered as a transcription of `verifyAuthorization`: an earlier
draft said "when, and only when" and then proved incomplete, so exhaustiveness is left to the code
and the items below are grouped by what each one actually governs.

*Conditions on the reference itself, checked by `verifyAuthorization`:*

1. it is authored by a principal whose `author_association` is `MEMBER`, `OWNER` or `COLLABORATOR`
   — `NONE` is never sufficient, and this is not relaxed;
2. it names the exact finding ID, the exact target state, and a non-empty reason;
3. it is bound to the repository and pull request number it applies to;
4. it is **not** carried by a review whose state is `DISMISSED`. `verifyAuthorization` refuses one
   today and 0008 established the rule expressly — a dismissed review returns the finding to
   `open`. An earlier draft omitted this condition entirely; the omission is recorded rather than
   quietly repaired.

*A temporal rule of the ordinary-finding path, applied in `evaluateTrustedReviewLoop` and not in
`verifyAuthorization`:*

5. the directive is issued strictly after the observation that reified or last changed the
   disposition of that finding, preserving the existing same-second comparator. This rule does
   **not** apply to `GENESIS` or to tombstones today, and this ADR does not extend it to them.

*Required persistence, an effect of evaluation rather than a precondition of it:*

6. the authorization is recorded in the journal with its reference identity, author and body
   digest, exactly as a third-party directive is today. A first evaluation may take the evidence
   from the current ledger, verify it and project the closure before `publishEvaluation` writes
   the record; the journal is the durable result, not a gate the directive passes through.

The only condition removed is the requirement that the directive's author differ from the pull
request author. Everything else that `verifyAuthorization` binds stays bound.

**The premise this relaxation rests on, and how it is kept honest.** The relaxation — under
either A or C — is justified by "one human", so the decision also fixes the guard that fails when
that stops being true. Under A the guard is the separate check specified below; under C the same
enumeration is consulted at authorization time instead.

**What is shared and what is not**, because an earlier draft of this paragraph said "the contract
is the same either way" and that is not true. Shared by both options: the counted set, the
authoritative source, the failure predicate, the completeness rule and the unsettled credential
question. **Not shared: the locus and cadence bullets below, which specify the Option A form.**
Under C the enumeration sits *on* the authorization path by design, and its load-bearing guarantee
is the per-disposal check rather than a per-evaluation run — so those two bullets are read as
"under A" and replaced under C by the mapping in verification item 9.

An earlier draft named the guard and left its design to the acceptance pull request; a reviewer
showed that this was not merely incomplete but unsound, because "it ships in the acceptance pull
request" says nothing about what *re-runs* it afterwards. A check that enumerates principals once,
during that pull request's CI, records the state of that day and then never fires again. The
premise would decay silently — exactly the failure the *Forces* requirement exists to prevent. So
the guard is decided here:

- **The set being counted, and the limit of the enumeration.** Principals who could satisfy
  condition 1: those whose `author_association` on this repository would be `OWNER`, `MEMBER` or
  `COLLABORATOR`. An earlier draft claimed the collaborators API "counts the same population
  condition 1 admits, so it cannot drift". **That equivalence is false in general and is
  withdrawn.** `author_association` describes a principal's relationship to the repository's owner;
  the collaborators endpoint enumerates repository access. In an organization-owned repository an
  organization member receives `MEMBER` on a comment without appearing as a collaborator, so the
  guard would count one while a second principal could satisfy condition 1. **The enumeration is
  therefore sound only for a personally-owned repository, which is what both repositories are
  today.** If either moves to an organization, this guard stops being sufficient and the decision
  must be revisited before it is relied on — recorded here rather than discovered later.
- **Authoritative source.** The repository collaborators API, `affiliation=all`, paginated to
  exhaustion by following `Link: rel="next"`. Repository metadata is the authority on who holds
  access; the pull request payload is not, because it reports only the associations of principals
  who happen to have acted.
- **The failure predicate.** The check fails when the enumeration contains any qualifying principal
  beyond the maintainer. Whether the owner appears in the enumeration must be **established against
  the API rather than assumed**, and the threshold derived from that fact: a guard that assumes the
  owner is listed, on an endpoint that does not list them, would sit at one principal when the
  second arrives and pass. That is the fail-open direction, and stating the predicate is what
  excludes it.
- **Completeness, and what happens without it.** The count is trustworthy only from an enumeration
  the check can show exhausted. A `403`, a rate-limited response, a transport failure, or a page
  sequence whose termination cannot be established yields `unknown` — never zero, never one. The
  check fails on `unknown`. Under-permissioning therefore fails closed rather than silently
  reporting an empty repository, which is the specific defect this wording exists to exclude.
- **Credentials — measured, not argued.** One thing was always settled: `administration` is
  **not** a valid key in a workflow's `permissions:` block, so an earlier draft specifying
  `metadata: read` plus `administration: read` was unimplementable at the locus it named. What was
  *not* settled was whether the evaluator's existing `GITHUB_TOKEN` can call the endpoint at all.
  Two readings of GitHub's documentation disagreed — one requiring *write, maintain or admin*
  privileges of the authenticated user, the other admitting **GitHub App installation access
  tokens** with repository `Metadata: read` — and an earlier draft asserted the first as fact and
  derived a requirement from it without verifying it. That assertion was retracted, and the
  question was made a precondition of acceptance rather than a better argument.

  **It was then measured.** A workflow declaring **the same five permissions as the evaluator**
  called the endpoint with `affiliation=all`, paginating to exhaustion
  ([keeplin#216](https://github.com/jsunyermias/keeplin/pull/216), run
  [31019731470](https://github.com/jsunyermias/keeplin/actions/runs/31019731470)):

  ```
  HTTP status: 200
  Pagination exhausted: true
  Principals returned: 1
  ```

  **The evaluator's existing `GITHUB_TOKEN` suffices. No new credential is required.** The second
  reading is the correct one: `GITHUB_TOKEN` is an installation access token, and the
  write/maintain/admin sentence governs the authenticated-*user* path. `Principals returned: 1`
  independently corroborates this ADR's answer to acceptance criterion 0 of keeplin#206.

  **What this does not establish.** It measures *this repository's configuration on the date of
  acceptance*. It is not a guarantee that survives a permissions change to the evaluator workflow,
  a repository transfer, or a move to an organization. That is precisely why verification item 9's
  fourth test performs the enumeration on every run instead of trusting this result — the same
  reasoning that moved the cadence guarantee off the scheduled leg. Under Option C the stake is
  higher than it was under A: the enumeration sits on the authorization path, so a credential that
  stops working refuses disposals rather than merely blocking a gate.
- **Locus — the *rejected* Option A form; see the status note.** In the default-branch evaluator workflow, alongside `check-review-loop.js` and
  **not** on the authorization path. Its failure blocks the gate; it never decides an individual
  disposal. **This bullet is the Option A form.** It is binding *under A*: an implementation that
  chose A and then consulted this count while verifying a directive has built C's placement under
  A's name, and the *Options considered* comparison no longer describes what was built. Under C
  that placement is the decision, not a defect.
- **Cadence, and which leg carries the guarantee — also the *rejected* Option A form.** Two triggers. **Every evaluator run** is the
  load-bearing one: self-authorization can only happen on a pull request, every pull request is
  evaluated, and so the premise is checked before every opportunity to use the relaxation. **A
  scheduled run on the default branch** is early warning, not the tripwire — it shortens the time
  a decayed premise goes unnoticed while no pull request is open. An earlier draft called the
  scheduled run "what makes the guard survive its own acceptance pull request", which overstated
  it: GitHub disables scheduled workflows in a public repository after 60 days without activity,
  and may delay or drop scheduled runs under load. A disabled schedule is still declared in the
  YAML, so its absence is not self-announcing. Nothing here fails closed when the job never starts
  — which is exactly why the guarantee is placed on the per-evaluation leg instead.

This is a decision about a check that does not exist yet, and it is recorded as a decision rather
than deferred because the acceptance pull request cannot be reviewed against an unstated contract.

**Invariants this establishes.**

- Disposal remains an explicit act with a named reason, never an inference from a table edit.
- Reification remains remembered: a finding that named a mechanical check cannot become
  `advisory` without a verified directive, and this ADR does not create a shortcut around that.
- The evaluator still refuses prose. A ledger row without a directive still projects to `open`.
- The bounds of 0008 and 0011 are unchanged and unweakened; this decision neither claims nor
  provides authenticity against a workflow sharing the App identity, nor detection of terminal
  truncation.

**Why the field narrows to A and C.** Option D is insufficient for the reason recorded above: it
removes at most the synthetic blocker. Option E leaves a rule that is routed around, which is
worse than a rule honestly relaxed.

Option B is the serious third alternative, and ruling it out does **not** rest on dismissing it.
Its capability boundary is real: a credential the agent cannot reach is materially different from
Option A, and compromising it does not by itself converge anything — the required jobs must still
be green, and a `resolved` still needs its success check bound to head, workflow, run and App. The
argument against B is narrower than an earlier draft of this ADR claimed, and both independent
reviewers caught that draft contradicting itself here. The argument is: the approval step recurs
on **every** disposal, not only genesis; the same person would stand on both sides of it in this
repository; and a step with no independent judgement behind it is worse than Option A precisely
because it *looks* like independent authorization. **That is a design judgement about this
project, not a measured finding**, and a maintainer who weighs the operational cost differently
should choose B.

**A against C — where this ADR stops.** C places the same live lookup on the authorization path,
which is not a new class of dependency since the evaluator already reads `author_association`
there. An earlier draft argued that this settled it for A, because A's failure mode is "a blocked
gate rather than a disposal wrongly permitted or wrongly refused". That argument does not hold and
is withdrawn. *Wrongly permitted* only arises if C is designed fail-open, and nothing forces that:
applying the `unknown ⇒ refuse` rule already decided above gives C a fail-closed lookup at no extra
machinery. What is left is *wrongly refused* against *blocked gate* — one disposal versus the whole
gate — and the smaller radius is C's. Add the decay behaviour, where C lapses safely and A does
not, and the written comparison favours C on both counts.

The honest summary: **A is simpler on the authorization path and worse in both failure events; C is
the reverse.** This ADR does not convert that into a recommendation, for the reason given at the
top of this section.

**What now defends against an agent self-disposing — stated precisely, because an earlier draft
overstated it.**

Nothing technical, **for an agent that controls a qualifying GitHub identity** — and in this
repository's tooling it does.

The precision matters, because an earlier draft offered the wrong evidence. Git commit authorship
(`Claude <noreply@anthropic.com>` on every commit of keeplin-srv#114) is Git metadata and proves
nothing about who can publish a review or comment under a qualifying association. The evidence
that does establish it: the agent-authored comments on that pull request are attributed to
`jsunyermias` with `author_association: OWNER`, because the tooling acts through the maintainer's
GitHub identity. That is the capability **the relaxation** stops filtering — under Option A
unconditionally, under Option C while the single-principal premise holds. It is not a capability C
retains: both options remove the author condition, and C only re-imposes it once the premise
fails.

**How far "eliminated" reaches, and where it stops.** An earlier draft argued that adversary 1's
defining capability — editing the ledger and the pull request body — implies a qualifying
identity, since GitHub grants it only to the author or a principal with write access. That
implication does not close, and the reviewer who caught it was right: **the author of a pull
request may edit its body whatever their association**, so a contributor opening one from a fork
with `author_association: NONE` is adversary 1 by the definition, *can* write the ledger, and is
still refused by condition 1 — which this ADR keeps and does not relax.

So the accurate statement is bounded:

- **Against adversary 1 as this repository instantiates it** — an agent acting through the
  maintainer's qualifying identity — the technical control is *eliminated*.
- **Against adversary 1 without a qualifying association** — a fork contributor authoring their
  own pull request — condition 1 still refuses the directive, and that residue is exactly what
  stops an outside contributor self-disposing findings on their own pull request.

The subset in the second row is empty in these repositories today, because one principal opens
every pull request. It stops being empty the moment any outside contributor opens one, and that
needs no new principal with a sufficient association.

**These are two different growth events, and conflating them would be a mistake.** An earlier
draft called them "the same condition". They are not. Condition 1 guards the second row
permanently and needs no check to do it: an outside contributor's directive is refused because of
their association, whatever else changes. The premise check — a separate guard under Option A, the
authorization-time enumeration under Option C — guards something else: the *single-human
justification* for relaxing the author/authorizer separation in the first place. It fires only
when a principal with a sufficient association appears. A reader who believed the
check covered outside contributors would relax their vigilance over condition 1, which is the one
thing still doing that work.

What remains is a record and a requirement, and neither is enforced by the evaluator:

- **The record.** A directive is dated, attributed and digest-bound, and the ledger row citing it
  is in the pull request body. As the table above states, **neither is in the file diff**: a
  reviewer reading the pull request sees them, a reviewer reading only `git diff` does not.
  `AGENTS.md` says "the ledger is part of the diff the independent reviewer examines"; that
  sentence is imprecise in the same way this ADR's earlier draft was, and correcting it belongs to
  a documentation issue, not here.
- **The requirement.** `AGENTS.md` requires an independent reviewer and forbids an agent from
  waiving one. This ADR does not touch that. But it is procedural: **no mechanism enforces it**,
  and `check-review-governance.js` runs inside the head-controlled `ci.yml`, which
  [0012](0012-default-branch-review-governance.md) already records as weakenable by a head.

So the honest formulation is: the defence moves from a mechanism to a convention, and the
convention's own enforcement is known to be incomplete. That is the cost of relaxing the author
condition at all — it falls on A and C alike — and the maintainer is accepting it knowingly or not
at all.

## Consequences and risks

**Positive.** Pull requests become able to converge, and findings become recordable as closed.

**Negative.** Against adversary 1 **as this repository instantiates it** — an agent acting through
the maintainer's qualifying identity — the technical control is eliminated, not reduced, for the
reason given under *Decision*: agents here act with the maintainer's credentials as a matter of
course. The qualifier is load-bearing and an earlier draft of this section dropped it: against
adversary 1 *without* a qualifying association, condition 1 still refuses the directive and this
decision changes nothing. Anyone quoting this line as "eliminated against adversary 1" is quoting
it wrong.

**Negative — branch protection cuts both ways.** Making `Review loop converged` a required check
becomes possible, and an earlier draft listed only that upside. The risk is the other half: it
would formalize as the sole *enforced* merge control precisely the gate this decision weakens,
while independent review — the control that actually replaces it — remains procedural and
unenforced. If the maintainer adds it, that asymmetry should be a conscious choice, not a side
effect of the gate finally being openable.

  **Decided at acceptance: not now.** The maintainer considered making `Review loop converged` a
  required check and declined for this reason — independent review, the control that actually
  replaces the separation, is still procedural, and `check-review-governance.js` runs inside the
  head-controlled `ci.yml` that [0012](0012-default-branch-review-governance.md) records as
  weakenable by a head. Recorded so a later reader knows the asymmetry was weighed rather than
  overlooked, and so revisiting it is a decision rather than a discovery.

**Residual risks.**

- The relaxation persists silently if a second principal joins and nobody revisits it. **Mitigated
  by the premise check, which this ADR both specifies under *Decision* and places in the
  acceptance pull request** — a separate guard under Option A, the authorization-time enumeration
  under Option C, with the same counted set, source, predicate and completeness rule either way — so the mitigation is a mechanism with a stated contract, not deferred
  work and not a named aspiration. Two earlier drafts fell short of that: the first called it
  follow-up here and a requirement elsewhere, leaving the acceptance boundary ambiguous; the second
  fixed the boundary but left the mechanism itself undecided, so this bullet claimed a mitigation
  while the verification plan admitted the check was not implementable. If the maintainer moves it
  out of acceptance, this risk returns unmitigated and the *Forces* requirement should be struck
  rather than left nominally satisfied.
- Directive fatigue: disposing of fourteen findings on one pull request requires fourteen
  directives. If that friction leads to batch-authorizing without reading, the audit trail records
  the act but not the absence of judgement.
- **keeplin-srv#114 is merged, and no mechanism described here clears it.** Directives bind to a
  pull request number, and nothing in the evaluator defines whether it runs on a closed pull
  request or how directives issued against one would produce a re-evaluation. Its
  `docs/review-stalls.md` row therefore stays open, and verification item 11 below is scoped to
  what the mechanism actually supports rather than asserting an outcome with no path to it.
  Defining the post-merge route is follow-up work, listed below.

**Observability — an explicit non-guarantee.** The journal records each disposal's reference,
author and digest, so the self-authorized/third-party split **is** derivable after the fact by
comparing each directive's recorded author against the pull request author. **What acceptance
does not deliver is any aggregate, readily visible signal**: nothing surfaces that split without
reconstructing it by hand, and therefore nothing makes it easy to notice the relaxation becoming the norm, fatigue
setting in, or abuse accumulating — precisely the failure modes that matter once the control moves
from prevention to after-the-fact detection. An earlier draft said "no new signal is required",
which contradicted the sentence that followed it. A counter on each journal record would close
this; whether it ships in the acceptance PR is the maintainer's call, and it is not assumed here.

**Part of the acceptance pull request, not follow-up.** The premise check — one that refuses, and
fails closed, when the repository gains another principal with sufficient association while
self-authorization is enabled. The *Forces* section requires this relaxation to "fail
loudly", and that force is unmet without it, so it belongs to the change rather than after it.
Its counted set, source, failure predicate and completeness rule are decided under *Decision*, so
the acceptance pull request implements a stated contract rather than inventing one; verification
item 9 states the four tests it must pass. **Under Option C the same enumeration is consulted at
authorization time instead of by a separate check**, so the locus and cadence bullets apply to A
and the rest applies to both. The credential is the one thing *Decision* deliberately leaves open,
and item 9's fourth test is what closes it.

**Follow-up work — genuinely later work only.**

- ~~A procedure, step by step, for issuing and recording a directive, linked from `AGENTS.md`.~~
  **Moved into the acceptance pull request** by maintainer decision: without it, whoever has to
  use the mechanism would deduce it from the code.
- **Correcting `AGENTS.md`.** Its sentence "the ledger is part of the diff the independent reviewer
  examines" is imprecise in the same way an earlier draft of this ADR was: the ledger is in the
  pull request body, not the file diff. This ADR flags it rather than editing it, and that flag
  would otherwise be an orphaned obligation — so it is listed here as work to be opened as a
  documentation issue.
- A defined post-merge route for already-merged pull requests carrying undisposed findings, so
  keeplin-srv#114's stall row has an exit.
- Clearing `docs/review-stalls.md` on both repositories through that exit.
- **The aggregate self-authorization counter, if the acceptance pull request does not carry it.**
  *Observability* leaves that to the maintainer rather than assuming it. Listed here so the option
  has a home either way; if it ships in acceptance, this entry is struck rather than left open.
- **A documentation anchor for the 1–5 / 6 distinction in `scripts/check-docs.sh`.** A reviewer
  asked for one and it is not in this pull request, because adding a mechanical check is
  implementation and this pull request implements nothing. Verification item 1 defends the
  distinction in prose; the anchor would defend it against a future edit. Recorded so the request
  is declined visibly rather than dropped.

## Compatibility, migration, and rollback

**Wire and format compatibility.** Not applicable. This decision touches neither the collab
protocol, `PROTOCOL_VERSION`, the `Change` model, format limits, the encryption envelope, nor any
persistent store. `keeplin-core` is unaffected and its pin does not move.

**Journal compatibility.** The journal record schema is unchanged by the decision itself. If the
aggregate counter of *Observability* is added — in acceptance or later, which this ADR leaves to
the maintainer rather than prejudging — it is a new digest-bound field on new records only;
existing records remain verifiable, exactly as `unauthenticatedAnchor` was introduced under 0013.

**Migration.** None. Directives issued after acceptance apply to findings already in a ledger; no
stored data is rewritten.

**Rollout ordering.** `check-review-loop.js` is shared byte-identically between the two
repositories, so the implementation lands as a coordinated pair of pull requests. Because the
evaluator runs from the default branch, the change takes effect for a repository only once merged
to its `main`, and the two repositories may briefly disagree. That window is benign: the stricter
behaviour is the current one.

**Rollback.** Reverting the implementation restores the current rule and returns every repository
to the state where no disposal verifies. Findings disposed of under this decision would project
back to `open`. Rollback is therefore safe but not free: it re-blocks anything that converged
under it.

## Verification plan

An earlier draft claimed "each item must fail if the corresponding behaviour is reverted" for the
whole list, which was not true of every item. The list is now grouped by what each item actually
establishes.

**Group 1 — regression of the decision itself.** These fail if the self-authorization change is
reverted.

1. **Positive.** A directive authored by the pull request author, satisfying conditions **1–5**,
   disposes of a reified finding and the blocking set shrinks — **and the evaluation then records
   it per condition 6**. Condition 6 is a postcondition here, not something the directive can
   satisfy in advance; an earlier draft of this item said "1–6" and re-flattened the very
   distinction the Decision section draws. The test must start with **no** such authorization in
   the journal, take the evidence from the current ledger, assert the projected closure, and
   capture the record `publishEvaluation` writes. Building it from a prior observation that
   already carries the authorization would pass without ever exercising the first evaluation this
   ADR promises. Test named in `.github/scripts/check-review-loop.test.js`.
2. **End to end — completed.** [keeplin#216](https://github.com/jsunyermias/keeplin/pull/216)
   issued repository disposal directive comment
   [5197775718](https://github.com/jsunyermias/keeplin/pull/216#issuecomment-5197775718), and check
   run `92457047341` published `converged`. Journal observation 2 recorded
   `unauthenticatedAnchor: false` and
   `blocking: 0`. This real execution satisfies keeplin#206's acceptance criterion 3.

**Group 2 — invariants that must survive the change, and one new guard.** Items 3–8 fail when
*their own* behaviour is reverted, not when self-authorization is; they pin what the decision
promises to leave untouched. Item 9 is different in kind — it pins a check that does **not** exist
yet — and is grouped here because it too fails on its own behaviour rather than on the decision's.

3. **Negative — association.** The same directive from an author with `author_association: NONE`
   is refused. This is the case the chosen policy rejects, and keeplin#206's acceptance criterion 5
   requires it to be covered by a named test.
4. **Negative — binding.** A directive naming a different finding, a different target state, a
   different pull request or a different repository is refused.
5. **Negative — ordering.** A directive issued in the same second as, or before, the observation
   that reified the finding is refused; the existing same-second comparator is unchanged.
6. **Negative — reification.** A reified finding cannot be moved to `advisory` without a
   verifying directive, and the retired-ID reservation still refuses a returning ID unreified.
7. **Negative — dismissed review.** A self-authorized directive carried by a review later set to
   `DISMISSED` returns the finding to `open`. This pins condition 4, which an earlier draft of
   this ADR omitted entirely.
8. **Failure injection.** With the authorization reference unreachable, the finding projects back
   to `reified: true, state: "open"` with its `disposalError`, exactly as today.

9. **Negative — the exception cannot outlive its premise.** With a second principal holding a
   sufficient association present, and self-authorization still enabled, **the exception is
   refused** — by the separate guard failing under Option A, by the authorization-time enumeration
   refusing the disposal under Option C. **And it fails closed**: an enumeration that is not
   demonstrably complete —
   pagination truncated, `403`, rate-limited, or any response the check cannot prove exhaustive —
   counts as *unknown*, never as zero principals. The check's counted set, source, failure
   predicate and completeness rule are specified under *Decision*; this item tests them. The
   credential is the one thing *Decision* leaves open, and the fourth test below is what closes it.
   **The first two tests:** a second principal present, and an inaccessible or partial response.

   **These tests were written in the Option A form and are remapped under Option C**, which is the
   policy the maintainer chose on acceptance. The mapping below is therefore not a menu: **the
   Option C column is binding and the Option A column is the record of the rejected alternative.**
   It is kept because a reviewer showed that test 3, read literally in its A form, would force a C
   implementation to build A's separate guard — the failure this mapping exists to prevent, and
   which is easier to avoid with both columns visible than with one deleted.

   The stimulus is the same in every row; **what differs is the observable**, and writing
   "unchanged" where only the stimulus is unchanged is what a reviewer caught here.

   | Test | Stimulus | Observable under A | Observable under C |
   |---|---|---|---|
   | 1 | a second qualifying principal exists | the separate guard fails and the gate blocks | the authorization-time enumeration refuses the disposal |
   | 2 | `403`, rate limit, transport failure, or non-exhaustive pagination | the separate guard yields `unknown` and the gate blocks | the authorization-time enumeration yields `unknown` and refuses **that disposal**; `verifyAuthorization` must not treat it as zero or one |
   | 3 | the workflow as configured | both triggers declared, guard on the per-evaluation leg | every directive verification consults the enumeration with `unknown ⇒ refuse`; scheduled and per-evaluation runs become optional early warning |
   | 4 | a real paginated call with the chosen credential | the expected set comes back | identical |

   **Row 2 is the one that must not be read loosely.** An implementation that chose C and kept a
   separate guard job merely to satisfy row 2 literally would prove that *the guard* fails on
   `403` while never proving that `verifyAuthorization` refuses the disposal — leaving C's
   authorization path free to treat `unknown` as zero and self-dispose with the suite green.

   **The first test is parameterised over both enumeration shapes** — owner present and owner
   absent — with a second principal in each, asserting failure in both. The predicate excludes the
   maintainer *by identity* rather than by a count, which is what makes it safe; the
   parameterisation is what proves an implementation actually did that, instead of hardcoding a
   threshold read off whichever shape its fixture happened to capture. Without it, the exact
   fail-open the predicate exists to exclude merges with the suite green.

   **Third test — the cadence *under Option A*; see the mapping above for its Option C form.** Two
   earlier drafts got this wrong in opposite directions. The
   first ran the check only in the acceptance pull request's CI, proving the state of that day and
   nothing after it. The second asserted that "a membership change with **no pull request open**
   still reaches the check" — **which no CI can assert**: it is an operational property of a
   scheduled trigger GitHub may disable or drop, not a mechanical one. This test is therefore
   scoped to what is actually assertable: that the workflow declares **both** triggers, that the
   guard runs on the per-evaluation path rather than only on the scheduled one. An implementation
   that fires only on `schedule` has put the guarantee on the leg that can silently stop.

   **Fourth test — the credential actually enumerates.**
   Test named
   `evaluator_GITHUB_TOKEN_really_enumerates_the_expected_repository_principals` in
   `.github/scripts/check-review-loop.test.js` is the original implementation of this test and
   has been present since this ADR was accepted. In each CI execution it performs the real
   paginated enumeration with that run's `GITHUB_TOKEN` against the worktree copy of the
   enumerator and asserts the repository-specific literal `["jsunyermias"]`; locally it skips
   when CI or the token is absent. Its failure belongs to the required `Check, Test & Lint` job.

   [`.github/workflows/adr-0015-verification.yml`](../../.github/workflows/adr-0015-verification.yml)
   adds a push-to-default-branch, daily scheduled, and manually dispatched probe, so the same
   credential property is checked daily even when repository activity does not start CI. It
   derives the expected singleton from `repository.owner.login` and exercises the enumerator
   fetched from the API-reported default branch, the same source used by the evaluator. Its
   failure belongs to its own job outside the required-job set. A static assertion that either
   runner "holds a credential" proves nothing: it passes for a credential that returns `403` on
   every call. The live calls assert that the expected set comes back and settle the open question
   *Decision* records: acceptance cannot claim a credential decision it has not exercised.

   **A tension this creates, stated rather than buried.** An *earlier draft* of *Options
   considered* charged Option C with needing a live lookup and credited Option A with reaching the
   same detectability without one; **the current text does neither**, and this paragraph asserted
   otherwise for one revision after that section was corrected. Adopting this check gives Option A
   a live lookup too. The
   distinction is narrow and must be held: **Option C's lookup sits on the authorization path**, so
   its failure refuses an individual disposal; **Option A's does not**, so its failure blocks the
   gate. *Decision* records that this comparison does **not** favour A — one disposal is a smaller
   radius than the whole gate — and therefore makes the placement a property of whichever option is
   chosen rather than an argument for either. Under A the lookup must stay off the authorization
   path; under C it must sit on it with `unknown ⇒ refuse`.

**Group 3 — deployment symmetry.** Byte-identity alone proves symmetry, not policy: if both
repositories reverted the change together it would still pass. It is therefore paired with a
behavioural check.

10. **Cross-repository.** The governance files remain byte-identical between the two repositories,
   **and** the Group 1 positive test passes when run against each repository's copy of
   `check-review-loop.js` independently.

**Group 4 — operational closure.** Moving a Markdown row is an administrative act that persists
whether or not the mechanism still works, so it cannot stand alone as evidence.

11. **Operational.** For each `docs/review-stalls.md` row moved to `Cleared`, a reproducible
    evaluation of the pull request it names no longer projects its blockers to `open`. The row
    move is the record of that, not the proof. **Scoped deliberately**: keeplin-srv#114 is merged
    and no route exists yet for re-evaluating a closed pull request, so this item covers pull
    requests the evaluator can still evaluate. keeplin#206's criterion 4 is met only once the
    post-merge route in *Follow-up work* exists.

Not verifiable, and stated rather than omitted: nothing here demonstrates authenticity against a
repository workflow carrying the same App identity, or detection of terminal truncation. Those
bounds belong to 0008 and 0011 and this plan does not test what it cannot establish.

## Equivalent decision in the other repository

This ADR is canonical in `jsunyermias/keeplin`, per the registry rule that cross-repository
decisions live here. `keeplin-srv` carries no copy; its `docs/adr/README.md` links this record.
The implementation is a coordinated pair of pull requests, because `check-review-loop.js` and its
workflow are byte-identical across both repositories and the symmetry check enforces that. No
`keeplin-core` pin or version implication follows, since no shared wire or format surface is
involved.
