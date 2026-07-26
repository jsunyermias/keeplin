---
name: Implementation issue
about: A prepared, implementable issue produced with docs/prompts/0.A-prompt-comun.md
title: "<area>: <one-line summary>"
labels: []
---

<!-- Prepared with `docs/prompts/0.A-prompt-comun.md`, which owns the rules this template
     only lays out. Delete every guidance comment before filing.
     Do not restate the roadmap position here: it lives in the `orden-NN` label. -->

| Field | Value |
|---|---|
| Severity | critical / high / medium / low, plus one clause on why |
| Cross-repo | `no`, or the companion issue and what must land in the same pair of PRs |
| Divisibility | phases (name the boundary), or `indivisible` (say what breaks if split) |
| ADR | `not required` with a reason, or `<repo> ADR NNNN` with its current status |
| Suggested implementer | model family or person, working from `0.B` |
| Reviewer | a family different from the implementer, working from `0.C` |
| Verified at | `<repo>@<sha>` that every factual claim below was checked against |

## Problem

<!-- What is wrong, as observed at the `Verified at` commit. Reference source by its
     `// md:` marker, never by line number. A pasted snippet is a dated observation,
     so pair it with the marker it came from. -->

## Objective

<!-- The bounded outcome. One paragraph. -->

## Affected files

<!-- Versioned paths, and the `// md:` markers inside them. Group by repository when the
     change is cross-repo. Companions of every listed source are in scope by default. -->

## Invariants and decisions

<!-- What must still hold afterwards, and the choices already settled. Name the precedent
     (an accepted ADR, an existing constant, a passing contract test) for each. -->

## Do not

<!-- Only issue-specific prohibitions that no check would stop: an adjacent bug that
     belongs to another `orden-NN`, a shared constant that may not move alone, a rename
     that is deferred. Leave out anything `check-docs.sh`, `clippy` or CI already blocks. -->

## Suggested context pack

<!-- The exact command and its measured estimate, not a hand-copied file list:

     ./scripts/context-pack <target> --list --profile understand|edit|review|cross-repo
     -> file_count N, estimated_tokens N (measured at the `Verified at` commit)

     Then the reading that no pack carries: AGENTS.md, this issue, the applicable ADR,
     and the companion of any file the change creates. -->

## Approach sketch — `UNVALIDATED DRAFT`

<!-- Optional. Validate it against the companions before writing code; if it contradicts
     a documented invariant, stop and report instead of implementing. Delete if absent. -->

## Acceptance criteria and verification

<!-- One row per criterion. A verifier is a named test, a check command, or the document
     section that must state it. "Manual review" is not a verifier. A test counts only if
     it fails when the fix is reverted. -->

| # | Acceptance criterion | Verifier |
|---|---|---|
| 1 | | |

## Dependencies

<!-- Blocking issues, the companion PR, and any decision that must be `accepted` first. -->
