# Review stalls

The implementation↔review loop converges mechanically: a pull request is done when its
required checks are green and no reified finding is open. When instead the loop stops making
progress, it escalates to the maintainer rather than iterating quietly. This file is the
durable record of what is currently stuck, so a loop that ran out of ideas stays visible
instead of being absorbed into another round.

The convergence rule, the blocking criterion and the stagnation brake are decided in
[`docs/adr/0004-review-loop-convergence.md`](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0004-review-loop-convergence.md) and enforced
by `.github/scripts/check-review-loop.js`.

## What counts as a stall

The loop state is hashed as `sha256(normalized diff ‖ open reified finding IDs ‖ red required
check names)`. A stall is either of:

- **A repeated state.** The current hash equals an earlier round's hash: the round changed
  nothing that matters to convergence.
- **No monotonic progress.** The blocking set `{red required checks} ∪ {open reified
  findings}` has not shrunk strictly for `REVIEW_LOOP_STAGNATION_LIMIT` rounds (3 by default).

The brake measures state, not elapsed time or token budget. A slow round that shrinks the
blocking set is progress; a fast round that does not is not.

## How an entry is opened

CI fails the pull request with the exact finding or check that is stuck and refuses to pass
until the stall is recorded here naming that pull request. An agent does not decide to
escalate and does not decide to keep going: continuing to iterate after a stall without this
record is prohibited. Write the entry in the same session the stall is detected.

An entry names what is stuck, not a narrative of the rounds. If the stuck item is a reified
finding, give its ID and the check that fails. If it is a red required check, name the check.

## How an entry is cleared

A stall is the maintainer's to resolve, and there are exactly three exits:

1. **Fix what fails** — the reified finding's check goes green, the blocking set shrinks, and
   the loop resumes normally.
2. **Dismiss the finding with a cited reason** — a priority decision or an accepted ADR. The
   finding moves to `dismissed` in the ledger with that citation and does not reopen unless the
   code in its area changes.
3. **Reclassify it as advisory** — only when it genuinely cannot be reduced to a failing
   check. It is then recorded, tracked as a follow-up issue, and no longer blocking.

Recording a stall here does not make the pull request mergeable, and neither does a green CI
run on an unrelated commit: convergence still requires zero open reified findings. Move the
entry to Cleared with the exit that was taken and a link to it.

## Open

| Detected | Pull request | Stuck on | Rounds without progress | Exit taken |
|---|---|---|---|---|
| — | — | — | — | — |

## Cleared

| Detected | Pull request | Stuck on | Exit | Resolution |
|---|---|---|---|---|
| — | — | — | — | — |
