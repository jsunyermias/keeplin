# Contributing to Keeplin

Read [`AGENTS.md`](AGENTS.md) before starting. It is the canonical engineering contract for
both Keeplin repositories.

## Contribution flow

1. Choose or create an issue with a bounded objective, dependencies and observable
   acceptance criteria. Resolve any required ADR through the
   [`docs/adr/` registry](docs/adr/README.md) before implementation; `proposed` is still a
   blocking state.
2. Start from current `main` and create a dedicated branch. Never commit directly to
   `main`.
3. Implement only the issue scope. Keep source, companion documents, Graphify corpus
   configuration and project documentation consistent. CI owns the ignored
   `graphify-out/` artifact; never commit it.
4. Run the applicable checks from `AGENTS.md` and open a draft PR. Link the issue and any
   companion PR in the other repository.
5. Complete the PR template's Author assertions with evidence. CI results belong only in
   the CI section.
6. Request a review from a human or model family different from the implementer. Give the
   reviewer the issue objective and diff; use
   [`docs/prompts/0.C-prompt-revision-seguridad.md`](docs/prompts/0.C-prompt-revision-seguridad.md)
   for adversarial review.
7. Resolve findings and conversations. Mark the PR ready only when the exact head commit has
   green required checks and independent review is recorded.
8. The maintainer merges. Update or rebase a stale branch and rerun required checks before
   merge; do not bypass protection or force-push `main`.

Accepted ADRs are historical records. A later architectural change creates a new ADR that
supersedes the old one; it does not rewrite the accepted decision to match a new diff.

## Cross-repository changes

A shared wire, format or behavior change is incomplete until coordinated PRs exist in both
repositories, `keeplin-srv` pins a green immutable `keeplin-core` revision, protocol
versions move in lockstep when breaking, and contract tests cover the boundary. Each PR
links its companion and neither is presented as independently complete.

## Branch protection contract

Repository administrators keep `main` protected in both `jsunyermias/keeplin` and
`jsunyermias/keeplin-srv` with:

- a pull request required before merging;
- required checks that must pass on the exact merge candidate;
- required resolution of review conversations;
- force-pushes and branch deletion disabled;
- administrator bypass reserved for incident recovery and documented if used.

For `jsunyermias/keeplin`, branch protection must require the `Check, Test & Lint` job that contains
the `Check filesystem format policy` step. Repository code can detect ordinary modification or
deletion of that gate after its introducing commit, but cannot ensure the workflow is selected as
a required check; confirming that setting is a maintainer action. Review of the introducing commit
must also validate the initial gate and release-latch contents, because immutability only protects
the bytes that introduction establishes.

Draft PRs are the default during implementation. The maintainer remains the only person who
performs the final merge in the normal workflow.

The repository does not currently use GitHub's merge queue. Before enabling it, add a
`merge_group` trigger to CI and give the filesystem-format policy range resolver a dedicated base
derivation for that event. A merge queue's synthetic merge commit supplies neither the push
event's `before` value nor a pull request's base SHA, so the existing derivations are not valid for
it.

A required check is identified by its job `name` in `.github/workflows/ci.yml`, so that name
is load-bearing configuration rather than a label. Renaming a required job without updating
the required-check list in Settings → Branches leaves protection waiting forever for a check
nobody reports: every pull request goes to `blocked` with all jobs green and no red signal
explaining it. Rename such a job only together with the settings update, in both
repositories, and say so in the pull request so the reviewer can confirm it.

## Prompt roles

- [`0.A-prompt-comun.md`](docs/prompts/0.A-prompt-comun.md): shared context and issue
  preparation.
- [`0.B-prompt-implementacion-issue.md`](docs/prompts/0.B-prompt-implementacion-issue.md):
  implementation from an accepted issue.
- [`0.C-prompt-revision-seguridad.md`](docs/prompts/0.C-prompt-revision-seguridad.md):
  independent, adversarial review.

Roles are defaults, not vendor restrictions. Review independence is mandatory.
