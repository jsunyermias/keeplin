"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const {
  DEFAULT_STAGNATION_LIMIT,
  STALLS_PATH,
  diffSignatureFromFiles,
  evaluateReviewLoop,
  loopStateHash,
  pendingChecksFromRuns,
  redChecksFromRuns,
  requiredChecksFromNeeds,
  splitTableRow,
  stallRecordsBlockers,
  DIRECTIVE_MARKER,
  evaluateTrustedReviewLoop,
  journalComment,
  makeJournalRecord,
  sha256,
} = require("./check-review-loop.js");

const REPOSITORY = "jsunyermias/keeplin";
const PULL_NUMBER = 200;
const DIFF = diffSignatureFromFiles([
  { filename: "keeplin-core/src/format.rs", status: "modified", sha: "aaaa111" },
]);

function ledger(findingRows, roundRows, extra = "") {
  return `## Review ledger

| ID | Round | Reified by | State | Resolution |
|---|---|---|---|---|
${findingRows.join("\n")}

### Round log

| Round | Loop-state hash | Blocking |
|---|---|---|
${roundRows.join("\n")}
${extra}
## Merge readiness
`;
}

function round(number, openReifiedIds, redChecks, blocking) {
  const hash = loopStateHash({ diffSignature: DIFF, openReifiedIds, redChecks });
  return `| ${number} | ${hash} | ${blocking} |`;
}

function evaluate(body, overrides = {}) {
  return evaluateReviewLoop({
    body,
    changedFiles: [],
    stallsContent: "",
    repository: REPOSITORY,
    pullNumber: PULL_NUMBER,
    redChecks: [],
    diffSignature: DIFF,
    ...overrides,
  });
}

// A body with no ledger section is round zero, not a malformed pull request — see the
// F-004 tests below. This suite therefore asserts the round-zero contract, not a rejection.

test("an empty ledger converges on green required checks alone", () => {
  const result = evaluate(ledger([], []));
  assert.equal(result.ok, true);
  assert.equal(result.state, "converged");
});

test("an empty ledger does not converge while a required check is red", () => {
  const result = evaluate(ledger([], []), { redChecks: ["Check, Test & Lint"] });
  assert.equal(result.ok, false);
  assert.equal(result.state, "converging");
  assert.match(result.message, /Check, Test & Lint/);
});

// Convergence: declared only with required checks green and zero open reified findings.

test("convergence needs zero open reified findings", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const result = evaluate(ledger(rows, [round(1, ["F-001"], [], 1)]));
  assert.equal(result.ok, false);
  assert.equal(result.state, "converging");
  assert.match(result.message, /F-001/);
});

test("convergence needs green required checks even with no open findings", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | resolved | fixed in abc1234 |`];
  const result = evaluate(ledger(rows, [round(1, [], ["Knowledge graph up to date"], 1)]), {
    redChecks: ["Knowledge graph up to date"],
  });
  assert.equal(result.ok, false);
  assert.equal(result.state, "converging");
});

test("convergence is declared with green checks and every finding disposed", () => {
  const rows = [
    `| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | resolved | fixed in abc1234 |`,
    `| F-002 | 1 | advisory | advisory | Naming preference; recorded, not blocking. |`,
    `| F-003 | 2 | advisory | dismissed | Out of scope per keeplin ADR 0003. |`,
  ];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, true);
  assert.equal(result.state, "converged");
  assert.match(result.message, /1 advisory/);
});

// The satisfied-reviewer condition is not accepted anywhere.

test("a ticked 'blocking findings are resolved' box does not converge an open reified finding", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const body = ledger(
    rows,
    [round(1, ["F-001"], [], 1)],
    "\n- [x] Blocking findings are resolved and conversations are closed.\n",
  );
  const result = evaluate(body);
  assert.equal(result.ok, false);
  assert.equal(result.state, "malformed");
  assert.match(result.message, /never a reviewer's satisfaction/i);
});

// Advisory: a finding with no failing check does not block the merge.

test("an advisory finding does not block convergence", () => {
  const rows = [
    `| F-001 | 1 | advisory | advisory | Readability concern; no check can express it. |`,
    `| F-002 | 1 | advisory | advisory | Prefer a different helper name. |`,
  ];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, true);
  assert.equal(result.state, "converged");
});

test("an open finding naming no failing check is rejected as unclassified", () => {
  const rows = [`| F-001 | 1 | advisory | open | |`];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, false);
  assert.equal(result.state, "malformed");
  assert.match(result.message, /open but names no failing check/i);
});

// Recurrence: a dismissed-with-reason finding does not restart the loop.

test("a dismissed finding with a cited reason does not block or restart the loop", () => {
  const rows = [
    `| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | dismissed | Deferred per keeplin ADR 0001; delivery gap is recorded there. |`,
  ];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, true);
  assert.equal(result.state, "converged");
});

test("re-raising a dismissed finding under its own ID keeps it dismissed", () => {
  const rows = [
    `| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | dismissed | Priority call: tracked by keeplin#151. |`,
    `| F-002 | 4 | advisory | advisory | Reviewer raised F-001 again in round 4; already dismissed, not reopened. |`,
  ];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, true);
  assert.equal(result.state, "converged");
});

test("a duplicate finding ID is rejected so a dismissal cannot be silently overwritten", () => {
  const rows = [
    `| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | dismissed | Priority call: tracked by keeplin#151. |`,
    `| F-001 | 4 | \`tests/collab.rs::rejects_replay\` | open | |`,
  ];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, false);
  assert.match(result.message, /appears more than once/i);
});

test("a dismissal without a cited reason is rejected", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | dismissed | |`];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, false);
  assert.match(result.message, /dismissed without a cited reason/i);
});

// Stagnation: escalate instead of iterating, and detect a repeated state by hash.

test("a repeated loop-state hash escalates and names the stuck item", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const repeated = round(1, ["F-001"], [], 1);
  const body = ledger(rows, [repeated, `| 2 | ${repeated.split("|")[2].trim()} | 1 |`]);
  const result = evaluate(body);
  assert.equal(result.ok, false);
  assert.equal(result.state, "escalated");
  assert.match(result.message, /byte-identical to round 1/i);
  assert.match(result.message, /F-001/);
});

// Churn without progress: the implementer pushes every round, so the diff — and therefore the
// loop-state hash — differs each time and the repeat rule never fires. Only the monotonic
// progress rule can catch this, which is why it exists alongside the hash.

test("a blocking set that does not shrink escalates at K rounds and not before", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const churned = (n, blocking) =>
    `| ${n} | ${loopStateHash({
      diffSignature: `round-${n}-diff`,
      openReifiedIds: ["F-001"],
      redChecks: [],
    })} | ${blocking} |`;
  const current = (n, blocking) =>
    `| ${n} | ${loopStateHash({
      diffSignature: DIFF,
      openReifiedIds: ["F-001"],
      redChecks: [],
    })} | ${blocking} |`;

  const beforeLimit = evaluate(ledger(rows, [churned(1, 3), churned(2, 3), current(3, 1)]));
  assert.equal(beforeLimit.state, "converging", beforeLimit.message);

  const atLimit = evaluate(
    ledger(rows, [churned(1, 1), churned(2, 1), churned(3, 1), current(4, 1)]),
  );
  assert.equal(atLimit.ok, false);
  assert.equal(atLimit.state, "escalated");
  assert.match(atLimit.message, new RegExp(`limit ${DEFAULT_STAGNATION_LIMIT}`));
  assert.match(atLimit.message, /F-001/);
});

test("an escalation demands a review-stalls entry naming this exact pull request", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const repeated = round(1, ["F-001"], [], 1);
  const hash = repeated.split("|")[2].trim();
  const body = ledger(rows, [repeated, `| 2 | ${hash} | 1 |`]);

  const missingFile = evaluate(body);
  assert.match(missingFile.message, new RegExp(`record it in ${STALLS_PATH}`, "i"));

  const openTable = (pull, stuck) =>
    `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n` +
    `| 2026-08-03 | https://github.com/${REPOSITORY}/pull/${pull} | ${stuck} |\n`;

  const wrongPull = evaluate(body, {
    changedFiles: [STALLS_PATH],
    stallsContent: openTable(199, "F-001"),
  });
  assert.equal(wrongPull.state, "escalated");
  assert.match(wrongPull.message, /must identify this exact pull request/i);

  const recorded = evaluate(body, {
    changedFiles: [STALLS_PATH],
    stallsContent: openTable(PULL_NUMBER, "F-001"),
  });
  assert.equal(recorded.ok, false, "a recorded stall is still not a merge");
  assert.equal(recorded.state, "escalated");
  assert.match(recorded.message, /escalated to the maintainer/i);
});

test("a stagnant loop that has actually converged is not escalated", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | resolved | fixed in abc1234 |`];
  const hash = loopStateHash({ diffSignature: DIFF, openReifiedIds: [], redChecks: [] });
  const body = ledger(rows, [
    `| 1 | ${hash} | 0 |`,
    `| 2 | ${hash} | 0 |`,
    `| 3 | ${hash} | 0 |`,
    `| 4 | ${hash} | 0 |`,
  ]);
  const result = evaluate(body);
  assert.equal(result.ok, true);
  assert.equal(result.state, "converged");
});

// The round log must describe the state CI actually observes.

test("a round log that does not record the current state is rejected", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, false);
  assert.equal(result.state, "malformed");
  assert.match(result.message, /current state hashes to/i);
});

test("a findings table with no round log is rejected and names the hash to record", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const result = evaluate(ledger(rows, []));
  assert.equal(result.ok, false);
  assert.match(result.message, /round log is empty/i);
  assert.match(result.message, /[0-9a-f]{64}/);
});

test("a mis-stated blocking count is rejected", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const hash = loopStateHash({ diffSignature: DIFF, openReifiedIds: ["F-001"], redChecks: [] });
  const result = evaluate(ledger(rows, [`| 1 | ${hash} | 5 |`]));
  assert.equal(result.ok, false);
  assert.match(result.message, /records a blocking set of 5/i);
});

test("an unstable finding ID is rejected", () => {
  const rows = [`| oops | 1 | advisory | advisory | . |`];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, false);
  assert.match(result.message, /stable finding ID/i);
});

test("an unknown finding state is rejected", () => {
  const rows = [`| F-001 | 1 | advisory | maybe | . |`];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, false);
  assert.match(result.message, /open, resolved, dismissed, advisory/i);
});

// The loop-state hash is a state function, not a clock.

test("the loop-state hash ignores ordering and depends on all three inputs", () => {
  const base = { diffSignature: DIFF, openReifiedIds: ["F-002", "F-001"], redChecks: ["b", "a"] };
  assert.equal(
    loopStateHash(base),
    loopStateHash({ diffSignature: DIFF, openReifiedIds: ["F-001", "F-002"], redChecks: ["a", "b"] }),
  );
  assert.notEqual(loopStateHash(base), loopStateHash({ ...base, diffSignature: "other" }));
  assert.notEqual(loopStateHash(base), loopStateHash({ ...base, openReifiedIds: ["F-001"] }));
  assert.notEqual(loopStateHash(base), loopStateHash({ ...base, redChecks: [] }));
});

test("the hash fields are separated, so content cannot migrate between them", () => {
  // Without a real separator these two states concatenate identically and collide, which
  // would let a changed diff masquerade as an unchanged one and defeat the stagnation brake.
  assert.notEqual(
    loopStateHash({ diffSignature: "diffF-001", openReifiedIds: [], redChecks: [] }),
    loopStateHash({ diffSignature: "diff", openReifiedIds: ["F-001"], redChecks: [] }),
  );
  assert.notEqual(
    loopStateHash({ diffSignature: "", openReifiedIds: ["F-001", "F-002"], redChecks: [] }),
    loopStateHash({ diffSignature: "", openReifiedIds: ["F-001"], redChecks: ["F-002"] }),
  );
});

test("the diff signature separates its own fields", () => {
  assert.notEqual(
    diffSignatureFromFiles([{ filename: "a", status: "b", sha: "c" }]),
    diffSignatureFromFiles([{ filename: "ab", status: "", sha: "c" }]),
  );
});

test("the normalized diff signature is commit-order independent but content sensitive", () => {
  const files = [
    { filename: "b.rs", status: "modified", sha: "222" },
    { filename: "a.rs", status: "added", sha: "111" },
  ];
  assert.equal(diffSignatureFromFiles(files), diffSignatureFromFiles([...files].reverse()));
  assert.notEqual(
    diffSignatureFromFiles(files),
    diffSignatureFromFiles([{ filename: "b.rs", status: "modified", sha: "333" }, files[1]]),
  );
});

test("F-014: diff ordering uses code-unit order", () => {
  const files = [
    { filename: "ä.rs", status: "modified", sha: "222" },
    { filename: "z.rs", status: "modified", sha: "111" },
  ];
  assert.equal(
    diffSignatureFromFiles(files),
    JSON.stringify([
      ["z.rs", "modified", "111"],
      ["ä.rs", "modified", "222"],
    ]),
  );
});

test("only completed failing check runs count as red, and the loop's own check is excluded", () => {
  const runs = [
    { name: "Check, Test & Lint", status: "in_progress", conclusion: null },
    { name: "Knowledge graph up to date", status: "completed", conclusion: "failure" },
    { name: "Audit", status: "completed", conclusion: "success" },
    { name: "Flaky", status: "completed", conclusion: "timed_out" },
    { name: "Self", status: "completed", conclusion: "failure" },
  ];
  assert.deepEqual(redChecksFromRuns(runs, ["Self"]), ["Flaky", "Knowledge graph up to date"]);
});

// Round 1 findings from the independent review (Codex / GPT-5.5, 2026-08-03).
// Each test below is the reification of one finding: it fails against the reviewed
// commit and passes once the finding is fixed.

// F-001 — convergence must require positive evidence that checks are green, never the
// mere absence of an already-completed failure.

test("F-001: an in-progress required check is not convergence", () => {
  const result = evaluate(ledger([], []), { pendingChecks: ["Check, Test & Lint"] });
  assert.equal(result.ok, false, "must not converge while a required check is still running");
  assert.notEqual(result.state, "converged");
  assert.match(result.message, /Check, Test & Lint/);
});

test("F-001: a queued check counts as not-green, not as green", () => {
  const runs = [
    { name: "Check, Test & Lint", status: "in_progress", conclusion: null },
    { name: "Knowledge graph up to date", status: "queued", conclusion: null },
    { name: "Audit", status: "completed", conclusion: "success" },
  ];
  assert.deepEqual(pendingChecksFromRuns(runs, []), [
    "Check, Test & Lint",
    "Knowledge graph up to date",
  ]);
  assert.deepEqual(pendingChecksFromRuns(runs, ["Check, Test & Lint"]), [
    "Knowledge graph up to date",
  ]);
});

test("F-001: every required needs result must be explicitly successful", () => {
  for (const result of ["skipped", "neutral", "cancelled", "unknown", undefined]) {
    assert.deepEqual(
      requiredChecksFromNeeds({ test: { result: "success" }, graph: { result } }),
      ["Knowledge graph up to date"],
    );
  }
  assert.deepEqual(
    requiredChecksFromNeeds({ test: { result: "success" }, graph: { result: "success" } }),
    [],
  );
  assert.deepEqual(requiredChecksFromNeeds({}), ["Check, Test & Lint", "Knowledge graph up to date"]);
});

test("F-001: unrelated optional checks are outside the explicit required-job set", () => {
  assert.deepEqual(
    requiredChecksFromNeeds({
      test: { result: "success" },
      graph: { result: "success" },
      optional: { result: "failure" },
    }),
    [],
  );
});

const TRUST = { repositoryId: 7, workflowId: 88, runId: 77, appSlug: "github-actions", appId: 15368 };
function journalFixture() {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 3 });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 2, priorDigest: first.digest });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 1, priorDigest: second.digest });
  return [first, second, third].map((record) => journalComment(record, TRUST));
}
function trusted(overrides = {}) {
  return evaluateTrustedReviewLoop({
    pull: { author: "author", headSha: "ccc", headRepositoryId: 7, baseRepositoryId: 7 },
    findings: [], references: [], checks: [], journalComments: journalFixture(),
    jobs: [
      { name: "Check, Test & Lint", status: "completed", conclusion: "success" },
      { name: "Knowledge graph up to date", status: "completed", conclusion: "success" },
    ], config: TRUST, ...overrides,
  });
}

test("journal accepts the shared intact fixture", () => assert.equal(trusted().state, "converged"));

test("edited journal record is history-unverifiable", () => {
  const comments = journalFixture();
  comments[1].body = comments[1].body.replace('"two"', '"changed"');
  assert.equal(trusted({ journalComments: comments }).state, "history-unverifiable");
});

test("middle-deleted journal record is history-unverifiable", () => {
  const comments = journalFixture();
  comments.splice(1, 1);
  assert.equal(trusted({ journalComments: comments }).state, "history-unverifiable");
});

test("limitation_F002_terminal_truncation_undetected", () => {
  const comments = journalFixture();
  const truncated = trusted({ journalComments: comments.slice(0, 2) });
  const prefixNeverExtended = trusted({ journalComments: comments.slice(0, 2) });
  assert.equal(truncated.state, "converged");
  assert.deepEqual(truncated.records.map((record) => record.stateHash), ["one", "two"]);
  assert.deepEqual(truncated.records, prefixNeverExtended.records);
  assert.doesNotMatch(JSON.stringify(truncated.records), /three/);
});

test("verified disposal requires exact directive, independent authorized author and body digest", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-013", state: "dismissed", reason: "ADR 0008" })} -->`;
  const reference = { id: 91, kind: "comment", user: { login: "maintainer" }, author_association: "MEMBER", body };
  const finding = { id: "F-013", reified: true, state: "dismissed", evidence: { referenceId: 91, author: "maintainer", bodyDigest: sha256(body) } };
  assert.equal(trusted({ findings: [finding], references: [reference] }).state, "converged");
  for (const changed of [
    { references: [] },
    { references: [{ ...reference, author_association: "CONTRIBUTOR" }] },
    { references: [{ ...reference, user: { login: "author" } }] },
    { references: [{ ...reference, body: `${body} edited` }] },
    { references: [{ ...reference, kind: "review", state: "DISMISSED" }] },
    { references: [{ ...reference, body: "looks good" }] },
  ]) assert.equal(trusted({ findings: [finding], ...changed }).projectedFindings[0].state, "open");
});

test("resolved evidence is bound to current head, named job, workflow and App", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-013", state: "resolved", reason: "test passes" })} -->`;
  const reference = { id: 92, kind: "comment", user: { login: "maintainer" }, author_association: "OWNER", body };
  const finding = { id: "F-013", reified: true, state: "resolved", evidence: { referenceId: 92, author: "maintainer", bodyDigest: sha256(body), checkRunId: 5, checkName: "Check, Test & Lint" } };
  const check = { id: 5, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };
  assert.equal(trusted({ findings: [finding], references: [reference], checks: [check] }).state, "converged");
  for (const mutation of [{ head_sha: "old" }, { workflow_id: 99 }, { workflow_run_id: 66 }, { app_id: 1 }, { conclusion: "neutral" }]) {
    assert.equal(trusted({ findings: [finding], references: [reference], checks: [{ ...check, ...mutation }] }).projectedFindings[0].state, "open");
  }
});

test("only named required jobs with positive success converge; forks fail closed", () => {
  for (const conclusion of ["skipped", "neutral", undefined, "failure"]) {
    const jobs = [{ name: "Check, Test & Lint", status: "completed", conclusion }, { name: "Knowledge graph up to date", status: "completed", conclusion: "success" }];
    assert.equal(trusted({ jobs }).state, "converging");
  }
  assert.equal(trusted({ jobs: [{ name: "optional", status: "completed", conclusion: "failure" }, ...trusted().records.slice(0, 0), { name: "Check, Test & Lint", status: "completed", conclusion: "success" }, { name: "Knowledge graph up to date", status: "completed", conclusion: "success" }] }).state, "converged");
  assert.equal(trusted({ pull: { author: "author", headSha: "ccc", headRepositoryId: 8, baseRepositoryId: 7 } }).state, "fork-refused");
});

test("genesis and tombstones require the same verified authorization", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "GENESIS", state: "genesis", reason: "migrate this PR" })} -->`;
  const reference = { id: 93, kind: "comment", user: { login: "maintainer" }, author_association: "COLLABORATOR", body };
  const evidence = { referenceId: 93, author: "maintainer", bodyDigest: sha256(body) };
  assert.equal(trusted({ journalComments: [], genesisEvidence: evidence, references: [reference] }).state, "converged");
  assert.equal(trusted({ journalComments: [], genesisEvidence: evidence, references: [] }).state, "history-unverifiable");
  const tombstoneBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-OLD", state: "tombstone", reason: "duplicate ID" })} -->`;
  const tombstoneReference = { id: 94, kind: "comment", user: { login: "maintainer" }, author_association: "MEMBER", body: tombstoneBody };
  const tombstone = { id: "F-OLD", evidence: { referenceId: 94, author: "maintainer", bodyDigest: sha256(tombstoneBody) } };
  assert.equal(trusted({ tombstones: [tombstone], references: [tombstoneReference] }).state, "converged");
  assert.equal(trusted({ tombstones: [tombstone], references: [] }).state, "history-unverifiable");
});

test("trusted workflow cannot execute or interpolate pull-request-head content", () => {
  const workflow = fs.readFileSync(".github/workflows/review-loop-evaluator.yml", "utf8");
  assert.match(workflow, /^\s*workflow_run:/m);
  assert.doesNotMatch(workflow, /actions\/checkout|^\s*run:|github\.event\.workflow_run\.head|github\.event\.pull_request|pull\.head\.ref|head_repository/m);
  const actions = workflow.match(/^\s*uses:\s*(.+)$/gm) || [];
  assert.deepEqual(actions.map((line) => line.trim()), ["uses: actions/github-script@60a0d83039c74a4aee543508d2ffcb1c3799cdea"]);
  assert.match(workflow, /ref: repository\.default_branch/);
  const evaluatorSource = fs.readFileSync(".github/scripts/check-review-loop.js", "utf8");
  assert.deepEqual(evaluatorSource.match(/require\([^\n]+/g), ['require("node:crypto");']);
});

test("every pull-request workflow is explicitly read-only and carries the 403 canary", () => {
  const workflows = fs.readdirSync(".github/workflows").filter((name) => /\.ya?ml$/.test(name));
  const pullWorkflows = workflows.map((name) => fs.readFileSync(`.github/workflows/${name}`, "utf8")).filter((body) => /^\s*pull_request:/m.test(body));
  assert.ok(pullWorkflows.length > 0);
  for (const workflow of pullWorkflows) {
    const permissions = workflow.match(/^permissions:\n((?:  [^\n]+\n)+)/m);
    assert.ok(permissions, "pull-request workflow must declare top-level permissions");
    assert.doesNotMatch(permissions[1], /:\s*write\s*$/m);
  }
  const ci = fs.readFileSync(".github/workflows/ci.yml", "utf8");
  assert.match(ci, /PATCH[\s\S]*check-runs\/\$\{check_id\}/);
  assert.match(ci, /test "\$\{status\}" = 403/);
});

// F-003 — the stall record must name what is stuck, not merely mention the pull request.

test("F-003: a stall record that names the PR but not the blocker is rejected", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const repeated = round(1, ["F-001"], [], 1);
  const hash = repeated.split("|")[2].trim();
  const body = ledger(rows, [repeated, `| 2 | ${hash} | 1 |`]);

  const vague = evaluate(body, {
    changedFiles: [STALLS_PATH],
    stallsContent: `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-08-03 | https://github.com/${REPOSITORY}/pull/${PULL_NUMBER} | something |\n`,
  });
  assert.equal(vague.state, "escalated");
  assert.match(vague.message, /F-001/, "must say the entry fails to name the stuck finding");
  assert.doesNotMatch(vague.message, /Recorded in/i);

  const named = evaluate(body, {
    changedFiles: [STALLS_PATH],
    stallsContent: `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-08-03 | https://github.com/${REPOSITORY}/pull/${PULL_NUMBER} | F-001 |\n`,
  });
  assert.equal(named.state, "escalated");
  assert.match(named.message, /escalated to the maintainer/i);
});

test("F-003: a cleared entry elsewhere in the file does not satisfy the record", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const repeated = round(1, ["F-001"], [], 1);
  const hash = repeated.split("|")[2].trim();
  const result = evaluate(ledger(rows, [repeated, `| 2 | ${hash} | 1 |`]), {
    changedFiles: [STALLS_PATH],
    stallsContent: `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| — | — | — |\n\n## Cleared\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-07-01 | ${REPOSITORY}#${PULL_NUMBER} | F-001 |\n`,
  });
  assert.equal(result.state, "escalated");
  assert.match(result.message, /Open/, "a Cleared row must not satisfy an open stall");
});

test("F-003: F-0010 is not an explicit token naming F-001", () => {
  const rows = [`| F-001 | 1 | \`tests/collab.rs::rejects_replay\` | open | |`];
  const repeated = round(1, ["F-001"], [], 1);
  const hash = repeated.split("|")[2].trim();
  const result = evaluate(ledger(rows, [repeated, `| 2 | ${hash} | 1 |`]), {
    changedFiles: [STALLS_PATH],
    stallsContent: `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-08-03 | ${REPOSITORY}#${PULL_NUMBER} | F-0010 |\n`,
  });
  assert.equal(result.state, "escalated");
  assert.match(result.message, /missing F-001/);
});

test("F-012: a comma-containing check name is one exact stall blocker", () => {
  const content = `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-08-03 | ${REPOSITORY}#${PULL_NUMBER} | Check, Test & Lint |\n`;
  assert.deepEqual(
    stallRecordsBlockers(content, REPOSITORY, PULL_NUMBER, ["Check, Test & Lint"]),
    { ok: true },
  );
});

test("F-012: br-delimited blocker names preserve punctuation and exact IDs", () => {
  const content = `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-08-03 | ${REPOSITORY}#${PULL_NUMBER} | Check, Test & Lint<br>F-0010; audit |\n`;
  assert.equal(
    stallRecordsBlockers(content, REPOSITORY, PULL_NUMBER, ["Check, Test & Lint", "F-001"]).ok,
    false,
  );
});

// F-004 — the ADR's migration contract: a pull request predating the ledger is round zero,
// not retroactively invalid.

test("F-004: a body with no ledger section is round zero, per ADR 0004", () => {
  const legacy = evaluate("## Summary\n\nA pull request opened before the ledger existed.");
  assert.equal(legacy.ok, true, "ADR 0004 says an absent ledger is round zero");
  assert.equal(legacy.state, "converged");
});

test("F-004: a legacy body still does not converge while a check is red", () => {
  const legacy = evaluate("## Summary\n\nLegacy.", { redChecks: ["Knowledge graph up to date"] });
  assert.equal(legacy.ok, false);
  assert.equal(legacy.state, "converging");
});

// F-005 — the hash must be injective; `Check, Test & Lint` contains a comma.

test("F-005: the hash does not collide on comma-containing names", () => {
  assert.notEqual(
    loopStateHash({ diffSignature: "d", openReifiedIds: [], redChecks: ["a,b", "c"] }),
    loopStateHash({ diffSignature: "d", openReifiedIds: [], redChecks: ["a", "b,c"] }),
  );
  assert.notEqual(
    loopStateHash({ diffSignature: "d", openReifiedIds: ["F-001,F-002"], redChecks: [] }),
    loopStateHash({ diffSignature: "d", openReifiedIds: ["F-001", "F-002"], redChecks: [] }),
  );
});

test("F-005: canonical framing remains injective for former delimiter bytes", () => {
  assert.notEqual(
    loopStateHash({ diffSignature: "d", openReifiedIds: ["a\x1fb", "c"], redChecks: [] }),
    loopStateHash({ diffSignature: "d", openReifiedIds: ["a", "b\x1fc"], redChecks: [] }),
  );
  assert.notEqual(
    diffSignatureFromFiles([{ filename: "a\0b", status: "", sha: "c" }]),
    diffSignatureFromFiles([{ filename: "a", status: "b", sha: "c" }]),
  );
});

// F-006 — an escaped pipe is legal Markdown and must not shift the columns.

test("F-006: an escaped pipe in a cell does not shift the state column", () => {
  const rows = [
    `| F-001 | 1 | \`tests/x.rs::accepts_a \\| b\` | dismissed | Superseded by contract A \\| B. |`,
  ];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, true, result.message);
  assert.equal(result.state, "converged");
});

test("F-006: CommonMark backslash parity controls whether a pipe separates cells", () => {
  const escaped = [`| F-001 | 1 | check \\| name | dismissed | reason \\| detail |`];
  assert.equal(evaluate(ledger(escaped, [round(1, [], [], 0)])).state, "converged");

  const separator = [`| F-001 | 1 | check \\\\| dismissed | trailing literal \\\\|`];
  assert.equal(evaluate(ledger(separator, [round(1, [], [], 0)])).state, "converged");

  const triple = [`| F-001 | 1 | check \\\\\\| name | dismissed | reason |`];
  assert.equal(evaluate(ledger(triple, [round(1, [], [], 0)])).state, "converged");
});

test("F-015: table parsing preserves an escaped terminal pipe and collapses backslash pairs", () => {
  assert.deepEqual(splitTableRow("| a \\\\ b | c |"), ["a \\ b", "c"]);
  const rows = [`| F-001 | 1 | advisory | advisory | terminal escaped pipe \\|`];
  const result = evaluate(ledger(rows, [round(1, [], [], 0)]));
  assert.equal(result.ok, true, result.message);
  assert.equal(result.state, "converged");
});

test("F-011: convergence claims only explicit workflow-dependency evidence", () => {
  const result = evaluate(ledger([], []));
  assert.equal(result.ok, true);
  assert.match(result.message, /explicit workflow dependencies succeeded/i);
  assert.doesNotMatch(result.message, /required checks are green/i);
});

// F-014 — ordering must be bytewise. localeCompare varies with the runtime's ICU version and
// locale, and ADR 0006 requires byte-identical results across both repositories.

test("F-014: open findings are ordered bytewise, not by locale", () => {
  const rows = [
    `| F-0010 | 1 | \`t::c\` | open | |`,
    `| F-002 | 1 | \`t::b\` | open | |`,
    `| F-001 | 1 | \`t::a\` | open | |`,
  ];
  const ids = ["F-0010", "F-002", "F-001"];
  const bytewise = [...ids].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const result = evaluate(ledger(rows, [round(1, ids, [], 3)]));
  const mentioned = bytewise.filter((id) => result.message.includes(id));
  assert.deepEqual(mentioned, bytewise, result.message);
  // The order the message reports must be the bytewise order, position for position.
  const positions = bytewise.map((id) => result.message.indexOf(id));
  assert.deepEqual([...positions].sort((a, b) => a - b), positions, result.message);
});

// Independent review stays a separate, conjunctive gate.

test("convergence says nothing about independent review", () => {
  const result = evaluate(ledger([], []));
  assert.equal(result.ok, true);
  assert.doesNotMatch(result.message, /independent review/i);
});
