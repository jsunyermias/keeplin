"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const EVALUATOR_PATH = process.env.REVIEW_LOOP_CHECK || "./check-review-loop.js";
const RECOVERY_PATH = process.env.REVIEW_LOOP_RECOVERY || ".github/scripts/check-review-loop-recovery.js";
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
  STATES,
  FAILING_CONCLUSIONS,
  AUTHORIZING_ASSOCIATIONS,
  AUTHORIZING_TRANSITIONS,
  REQUIRED_JOBS,
  DIRECTIVE_MARKER,
  JOURNAL_MARKER,
  enumerateRepositoryPrincipals,
  evaluateTrustedReviewLoop,
  journalComment,
  makeJournalRecord,
  parseFindings,
  publishEvaluation,
  recordedDispositionContext,
  requireParsedFindings,
  sha256,
  verifyAuthorization,
  verifyTerminalMalformedRecovery,
} = require(EVALUATOR_PATH);

const REPOSITORY = "jsunyermias/keeplin";
const PULL_NUMBER = 200;
const DIFF = diffSignatureFromFiles([
  { filename: "keeplin-core/src/format.rs", status: "modified", sha: "aaaa111" },
]);
const FAILED_DISPOSITION_REFUSAL_ANCHOR =
  "Failed-disposition refusal anchor: stop recovery and escalate to the maintainer.";
const STALLS_FAILED_DISPOSITION_DISCLOSURE =
  "The refusal anchor check guarantees the anchor's exact text, uniqueness, and position " +
  "immediately after the failed-disposition refusal paragraph. Positive prose assertions also " +
  "require that paragraph to identify the legacy candidate's projected values, state that the " +
  "failed disposition is not recoverable and the verifier must refuse it, forbid continued ledger " +
  "rewriting or refetching, and require maintainer escalation. They do not exclude contradictory " +
  "retry advice inside that paragraph.";
const SCRIPT_README_FAILED_DISPOSITION_DISCLOSURE =
  "The refusal anchor check guarantees the anchor's exact text, uniqueness, and position " +
  "immediately after the failed-disposition refusal paragraph. Positive prose assertions also " +
  "require that paragraph to identify the legacy evaluator projection, state that the failed " +
  "disposition is not recoverable by this procedure, and require maintainer escalation. They do " +
  "not exclude contradictory retry advice inside that paragraph.";

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

function paragraphContaining(markdown, pattern, description) {
  const paragraph = markdown.split(/\n\s*\n/).find((candidate) => pattern.test(candidate));
  assert.ok(paragraph, `documentation must contain ${description} in one paragraph`);
  return paragraph;
}

function failedDispositionRefusal(markdown) {
  return paragraphContaining(
    markdown,
    /terminal-malformed legacy record from\s+a failed disposition/i,
    "the failed-disposition refusal",
  );
}

function failedDispositionRefusalWithExactAnchor(markdown) {
  const refusal = failedDispositionRefusal(markdown);
  const anchoredRefusal = `${refusal}\n\n${FAILED_DISPOSITION_REFUSAL_ANCHOR}\n\n`;
  const refusalOffset = markdown.indexOf(refusal);
  const exactAnchorLines = markdown
    .split("\n")
    .filter((line) => line === FAILED_DISPOSITION_REFUSAL_ANCHOR);

  assert.equal(exactAnchorLines.length, 1, "the refusal anchor must occur once as an exact standalone line");
  assert.equal(
    markdown.slice(refusalOffset, refusalOffset + anchoredRefusal.length),
    anchoredRefusal,
    "the exact standalone refusal anchor must immediately follow the failed-disposition refusal paragraph",
  );
  return refusal;
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

const TRUST = { repositoryId: 7, repository: "jsunyermias/keeplin", pullNumber: 200, maintainerLogin: "maintainer", workflowId: 88, runId: 77, appSlug: "github-actions", appId: 15368 };
const COMPLETE_PRINCIPAL_ENUMERATION = { ok: true, principals: ["maintainer"] };
function journalFixture(length = 3) {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 3 });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 2, priorDigest: first.digest });
  if (length === 2) return [first, second].map((record) => journalComment(record, TRUST));
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 1, priorDigest: second.digest });
  return [first, second, third].map((record) => journalComment(record, TRUST));
}
function journalRecord(comment) {
  return JSON.parse(comment.body.slice(comment.body.indexOf("{"), comment.body.lastIndexOf("}") + 1));
}
function trusted(overrides = {}) {
  return evaluateTrustedReviewLoop({
    pull: { number: 200, author: "author", headSha: "ccc", headRepositoryId: 7, baseRepositoryId: 7 },
    findings: [], references: [], checks: [], journalComments: journalFixture(),
    jobs: [
      { name: "Check, Test & Lint", status: "completed", conclusion: "success" },
      { name: "Knowledge graph up to date", status: "completed", conclusion: "success" },
    ], principalEnumeration: COMPLETE_PRINCIPAL_ENUMERATION, config: TRUST, ...overrides,
  });
}

function trustedWorkflowScript(repositoryRoot) {
  const workflow = fs.readFileSync(path.join(repositoryRoot, ".github/workflows/review-loop-evaluator.yml"), "utf8");
  const lines = workflow.split("\n");
  const marker = lines.findIndex((line) => line.trim() === "script: |");
  assert.notEqual(marker, -1, "trusted evaluator workflow must contain an embedded script");
  const indentation = lines[marker].match(/^\s*/)[0].length + 2;
  return lines.slice(marker + 1)
    .map((line) => line.trim() === "" ? "" : line.slice(indentation))
    .join("\n");
}

async function executeTrustedWorkflow(repositoryRoot, pullBody, comments, options = {}) {
  const endpoints = {
    associatedPulls: async () => {},
    comments: async () => {},
    reviews: async () => {},
    jobs: async () => {},
    checks: async () => {},
    files: async () => {},
  };
  let postedBody;
  let reportedCheck;
  const github = {
    request: async () => ({ status: 200, data: [{ login: "maintainer" }], headers: {} }),
    paginate: async (endpoint, _parameters, mapPage) => {
      if (options.paginateError && endpoint === endpoints[options.paginateError.endpoint]) {
        throw options.paginateError.error;
      }
      if (endpoint === endpoints.associatedPulls) return [{ number: 200, state: "open", head: { sha: "ccc" } }];
      if (endpoint === endpoints.comments) return comments;
      if (endpoint === endpoints.checks && options.checkPages) {
        const items = [];
        for (const page of options.checkPages) {
          items.push(...[].concat(mapPage ? await mapPage(page) : page.data));
        }
        return items;
      }
      if (endpoint === endpoints.jobs && options.jobPages) {
        const items = [];
        for (const page of options.jobPages) {
          items.push(...[].concat(mapPage ? await mapPage(page) : page.data));
        }
        return items;
      }
      if (endpoint === endpoints.reviews || endpoint === endpoints.checks || endpoint === endpoints.files) return [];
      if (endpoint === endpoints.jobs) return [
        { name: "Check, Test & Lint", status: "completed", conclusion: "success" },
        { name: "Knowledge graph up to date", status: "completed", conclusion: "success" },
      ];
      throw new Error("unexpected paginated endpoint");
    },
    rest: {
      repos: {
        getContent: async ({ path: requestedPath }) => ({ data: { content: Buffer.from(
          requestedPath === ".github/scripts/check-review-loop.js"
            ? fs.readFileSync(path.join(repositoryRoot, requestedPath), "utf8")
            : "",
        ).toString("base64") } }),
        listPullRequestsAssociatedWithCommit: endpoints.associatedPulls,
      },
      pulls: {
        get: async () => {
          if (options.pullGetError) throw options.pullGetError;
          return { data: {
            number: 200,
            body: pullBody,
            user: { login: "author" },
            head: { sha: "ccc", repo: { id: options.headRepositoryId ?? 7 } },
            base: { repo: { id: 7 } },
          } };
        },
        listReviews: endpoints.reviews,
        listFiles: endpoints.files,
      },
      issues: {
        listComments: endpoints.comments,
        createComment: async ({ body }) => { postedBody = body; },
      },
      actions: {
        listJobsForWorkflowRun: endpoints.jobs,
        getWorkflowRun: async () => { throw new Error("no check lookup expected"); },
      },
      checks: {
        listForRef: endpoints.checks,
        create: async (check) => { reportedCheck = check; },
      },
    },
  };
  const context = {
    repo: { owner: "jsunyermias", repo: "keeplin" },
    payload: {
      repository: { id: 7, full_name: "jsunyermias/keeplin", default_branch: "main", owner: { login: "maintainer" } },
      workflow_run: { id: 77, run_attempt: 1, event: "pull_request", workflow_id: 88, head_sha: "ccc" },
    },
  };
  const failures = [];
  const core = { info: () => {}, setFailed: (message) => failures.push(message) };
  const priorWorkflowId = process.env.CI_WORKFLOW_ID;
  process.env.CI_WORKFLOW_ID = "88";
  try {
    const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
    await new AsyncFunction("github", "context", "core", "require", trustedWorkflowScript(repositoryRoot))(github, context, core, require);
  } finally {
    if (priorWorkflowId === undefined) delete process.env.CI_WORKFLOW_ID;
    else process.env.CI_WORKFLOW_ID = priorWorkflowId;
  }
  if (options.expectJournal === false) return { postedBody, reportedCheck, failures };
  if (options.expectFailure === false) assert.deepEqual(failures, [], "trusted workflow must not fail");
  assert.ok(postedBody, "trusted workflow must append a journal observation");
  return journalRecord({ body: postedBody });
}

function runRecoveryCommand(context, comments, finding, candidateCommentId) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "keeplin-recovery-input-"));
  context.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const commentsPath = path.join(directory, "comments.json");
  const pullPath = path.join(directory, "pull.json");
  fs.writeFileSync(commentsPath, JSON.stringify(comments));
  fs.writeFileSync(pullPath, JSON.stringify({ body: `## Review ledger\n\n| ID | Round | Reified by | State | Resolution |\n|---|---|---|---|---|\n| ${finding.id} | ${finding.round} | \`${finding.reifiedBy}\` | ${finding.state} | ${finding.resolution} |\n\n### Round log\n` }));
  return spawnSync(process.execPath, [
    RECOVERY_PATH,
    "--pull", pullPath,
    "--comments", commentsPath,
    "--candidate-comment-id", String(candidateCommentId),
    "--repository-id", String(TRUST.repositoryId),
    "--workflow-id", String(TRUST.workflowId),
    "--app-slug", TRUST.appSlug,
    "--app-id", String(TRUST.appId),
  ], { encoding: "utf8" });
}

function legacyFailedDispositionFixture() {
  const priorFinding = { id: "F-300", round: 3, reifiedBy: "mechanical check", reified: true, state: "open", resolution: "" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [priorFinding.id], findings: [priorFinding] });
  const prefix = { ...journalComment(first, TRUST), id: 1300, created_at: "2026-08-03T00:01:00Z" };
  const ledgerFinding = { id: "F-300", round: 4, reifiedBy: "advisory", reified: false, state: "advisory", resolution: "failed --> declassification" };
  const evaluated = trusted({ journalComments: [prefix], findings: [ledgerFinding], references: [] });
  const projected = evaluated.projectedFindings[0];
  const candidateRecord = makeJournalRecord({ ...TRUST, observation: 2, headSha: "ccc", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [projected.id], findings: [projected] });
  const candidate = { id: 1301, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(candidateRecord)} -->` };
  const replay = parseFindings(`| ID | Round | Reified by | State | Resolution |\n|---|---|---|---|---|\n| ${projected.id} | ${projected.round} | \`${projected.reifiedBy}\` | ${projected.state} | ${projected.resolution} |`);
  const parserDerivedReplay = [{ id: projected.id, round: projected.round, reifiedBy: projected.reifiedBy, reified: false, state: projected.state, resolution: projected.resolution }];
  return { prefix, candidate, projected, replay, parserDerivedReplay };
}

test("journal accepts the shared intact fixture", () => assert.equal(trusted().state, "converged"));

test("edited journal record is history-unverifiable", () => {
  const comments = journalFixture();
  comments[1].body = comments[1].body.replace('"two"', '"changed"');
  assert.equal(trusted({ journalComments: comments }).state, "history-unverifiable");
});

test("ledgerFindings mutation is rejected by the journal digest while the intact record verifies", () => {
  const finding = { id: "F-502", round: 1, reifiedBy: "digest mutation", reified: true, state: "open", resolution: "intact raw snapshot" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 1, findingIds: [finding.id], findings: [finding], ledgerFindings: [finding] });
  const tampered = { ...record, ledgerFindings: [{ ...finding, resolution: "tampered raw snapshot" }] };
  const intactResult = trusted({ journalComments: [journalComment(record, TRUST)], findings: [finding] });
  const tamperedResult = trusted({ journalComments: [journalComment(tampered, TRUST)], findings: [finding] });

  assert.notEqual(intactResult.state, "history-unverifiable", intactResult.message);
  assert.equal(tamperedResult.state, "history-unverifiable");
  assert.match(tamperedResult.message, /content does not match its carried digest/i);
});

test("unauthenticatedAnchor is digest-bound in both boolean directions", () => {
  for (const [recorded, tampered] of [[true, false], [false, true]]) {
    const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: recorded ? 1 : 0, unauthenticatedAnchor: recorded, findingIds: [], findings: [], ledgerFindings: [] });
    const intactComment = journalComment(record, TRUST);
    const tamperedComment = {
      ...intactComment,
      body: intactComment.body.replace(
        `"unauthenticatedAnchor":${recorded}`,
        `"unauthenticatedAnchor":${tampered}`,
      ),
    };

    assert.notEqual(tamperedComment.body, intactComment.body, "the fixture must flip the serialized flag");
    const rejected = trusted({ journalComments: [tamperedComment] });
    assert.equal(rejected.state, "history-unverifiable");
    assert.match(rejected.message, /content does not match its carried digest/i);

    const subsequent = trusted({ journalComments: [intactComment] });
    assert.equal(subsequent.unauthenticatedAnchor, recorded);
    assert.equal(subsequent.state, recorded ? "converging" : "converged");
    assert.equal(subsequent.syntheticFindings.some((finding) => finding.id === "GENESIS"), recorded);
  }
});

test("middle-deleted journal record is history-unverifiable", () => {
  const comments = journalFixture();
  comments.splice(1, 1);
  assert.equal(trusted({ journalComments: comments }).state, "history-unverifiable");
});

test("malformed_app_journal_marker_is_history_unverifiable", () => {
  const malformed = {
    id: 999,
    app_slug: TRUST.appSlug,
    app_id: TRUST.appId,
    body: `${JOURNAL_MARKER}{not-json} -->`,
  };
  const result = trusted({ journalComments: [...journalFixture(), malformed] });

  assert.equal(result.ok, false);
  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /malformed marker or JSON/i);
});

test("configured_app_comment_with_second_malformed_marker_is_history_unverifiable", () => {
  const comments = journalFixture();
  comments[0].body += `\n${JOURNAL_MARKER}{not-json} -->`;
  const result = trusted({ journalComments: comments });

  assert.equal(result.ok, false);
  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /malformed marker or JSON/i);
});

test("journal_field_containing_comment_terminator_round_trips_safely", () => {
  const finding = { id: "F-908", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 1, findingIds: [finding.id], findings: [finding] });
  const comment = journalComment(record, TRUST);
  const result = trusted({ journalComments: [comment], findings: [finding] });

  assert.notEqual(result.state, "history-unverifiable", result.message);
  assert.equal(comment.body.split(" -->").length - 1, 1, "only the framing delimiter may terminate the HTML comment");
  assert.equal(result.records[0].findings[0].resolution, finding.resolution);
});

test("raw_comment_terminator_inside_journal_json_still_fails_closed", () => {
  const finding = { id: "F-908", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 1, findingIds: [finding.id], findings: [finding] });
  const unsafe = { id: 1, app_slug: TRUST.appSlug, app_id: TRUST.appId, body: `${JOURNAL_MARKER}${JSON.stringify(record)} -->` };
  const result = trusted({ journalComments: [unsafe], findings: [finding] });

  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /malformed marker or JSON/i);
});

test("pre_fix_raw_comment_terminator_recovery_is_terminal_only_and_documented", () => {
  const scriptReadme = fs.readFileSync(".github/scripts/README.md", "utf8");
  const workflowCompanion = fs.readFileSync(".github/workflows/review-loop-evaluator.yml.md", "utf8");
  const stalls = fs.readFileSync("docs/review-stalls.md", "utf8");

  for (const claim of [scriptReadme, workflowCompanion]) {
    const compatibilityClaim = paragraphContaining(
      claim,
      /records written after this\s+change/i,
      "the forward-only delimiter compatibility claim",
    );
    assert.match(compatibilityClaim, /record written before this\s+change.*fails closed/is);
  }
  assert.match(stalls, /malformed record must be\s+terminal/i);
  assert.match(stalls, /authentic verifying prefix/i);
  assert.match(stalls, /re-recorded/i);
  const legacyAnchorClaim = paragraphContaining(
    workflowCompanion,
    /Legacy records without the field/i,
    "the absent unauthenticated-anchor interpretation",
  );
  assert.match(legacyAnchorClaim, /omission is read as authenticated/i);
  assert.match(legacyAnchorClaim, /App-identity digest boundary.*ADR\s+0011/is);
});

test("terminal_malformed_recovery_preserves_candidate_findings", () => {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [], findings: [] });
  const finding = { id: "F-101", round: 2, reifiedBy: "terminal_malformed_recovery_preserves_candidate_findings", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [finding.id], findings: [finding] });
  const comments = [
    { ...journalComment(first, TRUST), id: 1001, created_at: "2026-08-03T00:01:00Z" },
    { id: 1002, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(second)} -->` },
  ];
  const dropped = verifyTerminalMalformedRecovery({ comments, candidateCommentId: 1002, config: TRUST, replayFindings: [] });
  const preserved = verifyTerminalMalformedRecovery({ comments, candidateCommentId: 1002, config: TRUST, replayFindings: [finding] });

  assert.equal(dropped.ok, false);
  assert.match(dropped.message, /not semantically identical.*F-101/i);
  assert.equal(preserved.ok, true, preserved.message);
  assert.deepEqual(preserved.record.findings, [finding]);
});

test("terminal_malformed_recovery_reports_an_unauthenticated_anchor", (context) => {
  const finding = { id: "F-102", round: 1, reifiedBy: "anchor recovery", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 2, unauthenticatedAnchor: true, findingIds: [finding.id], findings: [finding], ledgerFindings: [finding] });
  const candidate = { id: 1012, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:01:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(record)} -->` };
  const recovered = verifyTerminalMalformedRecovery({ comments: [candidate], candidateCommentId: 1012, config: TRUST, replayFindings: [finding] });
  const command = runRecoveryCommand(context, [candidate], finding, 1012);

  assert.equal(recovered.ok, true, recovered.message);
  assert.equal(recovered.record.unauthenticatedAnchor, true);
  assert.equal(command.status, 0, command.stderr);
  assert.equal(JSON.parse(command.stdout).unauthenticatedAnchor, true);
});

test("a_non_boolean_unauthenticated_anchor_is_unreadable_in_evaluation_and_recovery", () => {
  const finding = { id: "F-103", round: 1, reifiedBy: "anchor shape", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 2, unauthenticatedAnchor: "yes", findingIds: [finding.id], findings: [finding], ledgerFindings: [finding] });
  const ordinary = journalComment(record, TRUST);
  const candidate = { id: 1013, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:01:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(record)} -->` };
  const evaluation = trusted({ journalComments: [ordinary], findings: [finding] });
  const recovery = verifyTerminalMalformedRecovery({ comments: [candidate], candidateCommentId: 1013, config: TRUST, replayFindings: [finding] });

  assert.equal(evaluation.state, "history-unverifiable");
  assert.match(evaluation.message, /unauthenticatedAnchor.*boolean/i);
  assert.equal(recovery.ok, false);
  assert.match(recovery.message, /unauthenticatedAnchor.*boolean/i);
});

test("terminal_malformed_recovery_accepts_safe_replay_of_evaluator_projected_open_finding", () => {
  const current = { id: "F-200", round: 4, reifiedBy: "invalid disposition", reified: true, state: "dismissed", resolution: "literal --> remains journal data" };
  const evaluated = trusted({ findings: [current], references: [] });
  const projected = evaluated.projectedFindings[0];
  assert.equal(projected.state, "open");
  assert.match(projected.disposalError, /authorization reference is unreachable/i);
  const prefix = journalFixture();
  const prior = journalRecord(prefix[prefix.length - 1]);
  const candidateRecord = makeJournalRecord({ ...TRUST, observation: 4, headSha: "ccc", stateHash: "four", blocking: 1, priorDigest: prior.digest, findingIds: [projected.id], findings: [projected] });
  const candidate = { id: 1004, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:04:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(candidateRecord)} -->` };
  const { disposalError, ...replayFinding } = projected;

  const safe = verifyTerminalMalformedRecovery({ comments: [...prefix, candidate], candidateCommentId: 1004, config: TRUST, replayFindings: [replayFinding] });
  const changedResolution = verifyTerminalMalformedRecovery({ comments: [...prefix, candidate], candidateCommentId: 1004, config: TRUST, replayFindings: [{ ...replayFinding, resolution: "different load-bearing ledger value" }] });

  assert.equal(safe.ok, true, safe.message);
  assert.equal(changedResolution.ok, false);
  assert.match(changedResolution.message, /not semantically identical.*F-200/i);
});

test("legacy_failed_disposition_candidate_is_refused_when_markdown_cannot_reproduce_its_projection", () => {
  const { prefix, candidate, projected, replay, parserDerivedReplay } = legacyFailedDispositionFixture();

  assert.equal(projected.reifiedBy, "advisory");
  assert.equal(projected.reified, true);
  assert.equal(projected.state, "open");
  assert.match(replay.error, /open but names no failing check/i);
  const refused = verifyTerminalMalformedRecovery({ comments: [prefix, candidate], candidateCommentId: 1301, config: TRUST, replayFindings: parserDerivedReplay });
  assert.equal(refused.ok, false);
  assert.match(refused.message, /not semantically identical.*F-300/i);
});

test("terminal_malformed_recovery_accepts_safe_replay_of_evaluator_projected_failed_declassification", () => {
  const priorFinding = { id: "F-300", round: 3, reifiedBy: "mechanical check", reified: true, state: "open", resolution: "" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [priorFinding.id], findings: [priorFinding] });
  const prefix = { ...journalComment(first, TRUST), id: 1300, created_at: "2026-08-03T00:01:00Z" };
  const replayFinding = { id: "F-300", round: 4, reifiedBy: "advisory", reified: false, state: "advisory", resolution: "literal --> remains journal data" };
  const evaluated = trusted({ journalComments: [prefix], findings: [replayFinding], references: [] });
  const projected = evaluated.projectedFindings[0];
  assert.equal(projected.reified, true);
  assert.equal(projected.state, "open");
  assert.match(projected.disposalError, /authorization reference is unreachable/i);
  const candidateRecord = makeJournalRecord({ ...TRUST, observation: 2, headSha: "ccc", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [projected.id], findings: [projected], ledgerFindings: [replayFinding] });
  const candidate = { id: 1301, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(candidateRecord)} -->` };

  const recovered = verifyTerminalMalformedRecovery({ comments: [prefix, candidate], candidateCommentId: 1301, config: TRUST, replayFindings: [replayFinding] });

  assert.equal(recovered.ok, true, recovered.message);
});

test("terminal_malformed_recovery_refuses_semantically_different_failed_declassification_replay", () => {
  const priorFinding = { id: "F-300", round: 3, reifiedBy: "mechanical check", reified: true, state: "open", resolution: "" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [priorFinding.id], findings: [priorFinding] });
  const prefix = { ...journalComment(first, TRUST), id: 1300, created_at: "2026-08-03T00:01:00Z" };
  const replayFinding = { id: "F-300", round: 4, reifiedBy: "advisory", reified: false, state: "advisory", resolution: "literal --> remains journal data" };
  const evaluated = trusted({ journalComments: [prefix], findings: [replayFinding], references: [] });
  const projected = evaluated.projectedFindings[0];
  const candidateRecord = makeJournalRecord({ ...TRUST, observation: 2, headSha: "ccc", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [projected.id], findings: [projected], ledgerFindings: [replayFinding] });
  const candidate = { id: 1301, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(candidateRecord)} -->` };

  const refused = verifyTerminalMalformedRecovery({ comments: [prefix, candidate], candidateCommentId: 1301, config: TRUST, replayFindings: [{ ...replayFinding, state: "dismissed" }] });

  assert.equal(refused.ok, false);
  assert.match(refused.message, /not semantically identical.*F-300/i);
});

test("terminal_malformed_recovery_refuses_dismissed_candidate_replayed_as_advisory", () => {
  const priorFinding = { id: "F-400", round: 3, reifiedBy: "mechanical check", reified: true, state: "open", resolution: "" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [priorFinding.id], findings: [priorFinding] });
  const prefix = { ...journalComment(first, TRUST), id: 1400, created_at: "2026-08-03T00:01:00Z" };
  const dismissedFinding = { id: "F-400", round: 4, reifiedBy: "advisory", reified: false, state: "dismissed", resolution: "literal --> remains journal data" };
  const evaluated = trusted({ journalComments: [prefix], findings: [dismissedFinding], references: [] });
  const projected = evaluated.projectedFindings[0];
  assert.equal(projected.reified, true);
  assert.equal(projected.state, "open");
  assert.match(projected.disposalError, /authorization reference is unreachable/i);
  const candidateRecord = makeJournalRecord({ ...TRUST, observation: 2, headSha: "ccc", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [projected.id], findings: [projected], ledgerFindings: [dismissedFinding] });
  const candidate = { id: 1401, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(candidateRecord)} -->` };
  const advisoryReplay = { ...dismissedFinding, state: "advisory" };

  const recovered = verifyTerminalMalformedRecovery({ comments: [prefix, candidate], candidateCommentId: 1401, config: TRUST, replayFindings: [advisoryReplay] });

  assert.equal(recovered.ok, false);
  assert.match(recovered.message, /not semantically identical.*F-400/i);
});

test("terminal_malformed_recovery_excludes_authorized_advisory_from_failed_declassification_recovery", () => {
  const priorFinding = { id: "F-403", round: 3, reifiedBy: "mechanical check", reified: true, state: "open", resolution: "" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [priorFinding.id], findings: [priorFinding] });
  const prefix = { ...journalComment(first, TRUST), id: 1430, created_at: "2026-08-03T00:01:00Z" };
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-403", state: "advisory", reason: "maintainer declassification" })} -->`;
  const advisoryFinding = { id: "F-403", round: 4, reifiedBy: "advisory", reified: false, state: "advisory", resolution: "authorized --> declassification", evidence: { referenceId: 403, author: "maintainer", bodyDigest: sha256(body) } };
  const reference = { id: 403, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body, created_at: "2026-08-03T00:02:00Z" };
  const evaluated = trusted({ journalComments: [prefix], findings: [advisoryFinding], references: [reference] });
  const projected = evaluated.projectedFindings[0];
  assert.equal(projected.reified, false);
  assert.equal(projected.state, "advisory");
  assert.equal(projected.disposalError, undefined);
  const candidateRecord = makeJournalRecord({ ...TRUST, observation: 2, headSha: "ccc", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: [projected.id], findings: [projected], ledgerFindings: [advisoryFinding] });
  const candidate = { id: 1431, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:03:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(candidateRecord)} -->` };

  const recovered = verifyTerminalMalformedRecovery({ comments: [prefix, candidate], candidateCommentId: 1431, config: TRUST, replayFindings: [advisoryFinding] });

  assert.equal(recovered.ok, true, recovered.message);
});

test("terminal_malformed_recovery_rejects_unaccounted_suffix_and_accepts_whitespace", () => {
  const finding = { id: "F-213", round: 4, reifiedBy: "recovery suffix", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 1, findingIds: [finding.id], findings: [finding] });
  const candidate = (suffix) => ({ id: 1013, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:01:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(record)} -->${suffix}` });

  const unaccounted = verifyTerminalMalformedRecovery({ comments: [candidate("\nmaterial the verifier did not account for")], candidateCommentId: 1013, config: TRUST, replayFindings: [finding] });
  const whitespace = verifyTerminalMalformedRecovery({ comments: [candidate(" \n\t")], candidateCommentId: 1013, config: TRUST, replayFindings: [finding] });

  assert.equal(unaccounted.ok, false);
  assert.match(unaccounted.message, /content after the recoverable record/i);
  assert.equal(whitespace.ok, true, whitespace.message);
});

test("terminal_malformed_recovery_rejects_forked_predecessor_and_accepts_continuity", () => {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [], findings: [] });
  const finding = { id: "F-102", round: 2, reifiedBy: "recovery continuity", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const matching = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [finding.id], findings: [finding] });
  const forked = makeJournalRecord({ ...matching, priorDigest: "0".repeat(64), digest: undefined });
  const prefix = { ...journalComment(first, TRUST), id: 1001, created_at: "2026-08-03T00:01:00Z" };
  const candidate = (record) => ({ id: 1002, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(record)} -->` });

  const refused = verifyTerminalMalformedRecovery({ comments: [prefix, candidate(forked)], candidateCommentId: 1002, config: TRUST, replayFindings: [finding] });
  const accepted = verifyTerminalMalformedRecovery({ comments: [prefix, candidate(matching)], candidateCommentId: 1002, config: TRUST, replayFindings: [finding] });

  assert.equal(refused.ok, false);
  assert.match(refused.message, /priorDigest.*verified surviving head/i);
  assert.equal(accepted.ok, true, accepted.message);
});

test("terminal_malformed_recovery_documentation_is_executable_and_identifies_configuration_sources", () => {
  const stalls = fs.readFileSync("docs/review-stalls.md", "utf8");
  const configuration = paragraphContaining(
    stalls,
    /`repositoryId` comes from the repository API/i,
    "the recovery configuration sources",
  );
  const deletionGate = paragraphContaining(
    stalls,
    /Do not delete anything unless the verifier exits zero/i,
    "the zero-exit deletion gate",
  );

  assert.match(stalls, /check-review-loop-recovery\.js/);
  assert.match(stalls, /gh api --paginate/);
  assert.match(configuration, /repositoryId.*repository API/is);
  assert.match(configuration, /workflowId.*CI_WORKFLOW_ID/is);
  assert.match(configuration, /appSlug.*appId.*default-branch.*review-loop-evaluator\.yml/is);
  assert.match(deletionGate, /exit(?:s| status).*zero/is);
});

test("terminal_malformed_recovery_documentation_pins_raw_snapshot_and_exact_failed_disposition_refusal_anchor", () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const stalls = fs.readFileSync(path.join(repositoryRoot, "docs/review-stalls.md"), "utf8");
  const snapshot = paragraphContaining(
    stalls,
    /Before deletion,\s+restore the pull request's current review ledger/i,
    "the pre-deletion snapshot source",
  );
  const refusal = failedDispositionRefusalWithExactAnchor(stalls);
  const deletionGate = paragraphContaining(
    stalls,
    /Do not delete anything unless the verifier exits zero/i,
    "the zero-exit deletion gate",
  );
  const disclosure = paragraphContaining(
    stalls,
    /The refusal anchor check guarantees the anchor's exact text/i,
    "the exact scope of the failed-disposition documentation checks",
  );

  assert.match(snapshot, /raw pre-projection ledger.*ledgerFindings/is);
  assert.match(refusal, /legacy candidate without `ledgerFindings`.*projected values/is);
  assert.match(refusal, /failed disposition is not recoverable.*verifier must\s+refuse it/is);
  assert.match(refusal, /Do not keep rewriting or refetching the ledger.*escalate.*maintainer/is);
  assert.equal(disclosure.replaceAll("\n", " "), STALLS_FAILED_DISPOSITION_DISCLOSURE);
  assert.match(deletionGate, /zero exit.*current ledger.*identified by `findingsSource`/is);
  assert.doesNotMatch(stalls, /reified`, which is derived deterministically from\s+`reifiedBy`/i);
  assert.doesNotMatch(stalls, /preserves every candidate\s+finding exactly/i);
});

test("legacy_recovery_documentation_matches_fallback_output_and_pins_exact_refusal_anchor", () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const stalls = fs.readFileSync(path.join(repositoryRoot, "docs/review-stalls.md"), "utf8");
  const scriptReadme = fs.readFileSync(path.join(repositoryRoot, ".github/scripts/README.md"), "utf8");
  const { prefix, candidate, projected, replay, parserDerivedReplay } = legacyFailedDispositionFixture();
  const recovery = verifyTerminalMalformedRecovery({ comments: [prefix, candidate], candidateCommentId: 1301, config: TRUST, replayFindings: parserDerivedReplay });

  assert.equal(projected.reified, true);
  assert.match(replay.error, /open but names no failing check/i);
  assert.equal(recovery.ok, false, "the documented Markdown replay cannot reproduce the legacy projection");
  for (const runbook of [stalls, scriptReadme]) {
    const refusal = failedDispositionRefusalWithExactAnchor(runbook);
    assert.match(refusal, /failed disposition is not recoverable by this procedure/is);
    assert.match(refusal, /legacy (?:candidate.*projected values|evaluator\s+projection)/is);
    assert.match(refusal, /escalate.*maintainer|maintainer escalation/is);
  }
  const stallsDisclosure = paragraphContaining(
    stalls,
    /The refusal anchor check guarantees the anchor's exact text/i,
    "the review-stalls scope of the failed-disposition documentation checks",
  );
  const scriptReadmeDisclosure = paragraphContaining(
    scriptReadme,
    /The refusal anchor check guarantees the anchor's exact text/i,
    "the script README scope of the failed-disposition documentation checks",
  );
  assert.equal(stallsDisclosure.replaceAll("\n", " "), STALLS_FAILED_DISPOSITION_DISCLOSURE);
  assert.equal(scriptReadmeDisclosure.replaceAll("\n", " "), SCRIPT_README_FAILED_DISPOSITION_DISCLOSURE);
});

test("trusted_workflow_journals_raw_pre_projection_findings_for_recovery", async () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const priorFinding = { id: "F-501", round: 1, reifiedBy: "mechanical check", reified: true, state: "open", resolution: "" };
  const priorRecord = makeJournalRecord({ ...TRUST, runId: 76, runAttempt: 1, observation: 1, headSha: "bbb", stateHash: "prior", blocking: 1, findingIds: [priorFinding.id], findings: [priorFinding] });
  const comments = [{ ...journalComment(priorRecord, TRUST), id: 1501, created_at: "2026-08-03T00:01:00Z", performed_via_github_app: { slug: TRUST.appSlug, id: TRUST.appId } }];
  const pullBody = `## Review ledger\n\n| ID | Round | Reified by | State | Resolution |\n|---|---|---|---|---|\n| F-501 | 2 | \`advisory\` | advisory | failed declassification |\n\n### Round log\n`;
  const record = await executeTrustedWorkflow(repositoryRoot, pullBody, comments);

  assert.deepEqual(record.ledgerFindings, [{ id: "F-501", round: 2, reifiedBy: "advisory", reified: false, state: "advisory", resolution: "failed declassification" }]);
  assert.equal(record.findings[0].id, "F-501");
  assert.equal(record.findings[0].reified, true);
  assert.equal(record.findings[0].state, "open");
  assert.notDeepEqual(record.ledgerFindings, record.findings, "workflow must journal the raw parser result, not the lossy evaluator projection");
});

test("terminal_malformed_recovery_command_executes_the_mechanical_check", (context) => {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [], findings: [] });
  const finding = { id: "F-101", round: 2, reifiedBy: "recovery command", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [finding.id], findings: [finding] });
  const comments = [
    { ...journalComment(first, TRUST), id: 1001, created_at: "2026-08-03T00:01:00Z" },
    { id: 1002, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(second)} -->` },
  ];
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "keeplin-recovery-"));
  context.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const commentsPath = path.join(directory, "comments.json");
  const pullPath = path.join(directory, "pull.json");
  fs.writeFileSync(commentsPath, JSON.stringify(comments));
  fs.writeFileSync(pullPath, JSON.stringify({ body: `## Review ledger\n\n| ID | Round | Reified by | State | Resolution |\n|---|---|---|---|---|\n| F-101 | 2 | \`recovery command\` | open | literal --> remains journal data |\n\n### Round log\n` }));
  const command = spawnSync(process.execPath, [
    RECOVERY_PATH,
    "--pull", pullPath,
    "--comments", commentsPath,
    "--candidate-comment-id", "1002",
    "--repository-id", String(TRUST.repositoryId),
    "--workflow-id", String(TRUST.workflowId),
    "--app-slug", TRUST.appSlug,
    "--app-id", String(TRUST.appId),
  ], { encoding: "utf8" });

  assert.equal(command.status, 0, command.stderr);
  const output = JSON.parse(command.stdout);
  assert.equal(output.findingsSource, "legacy evaluator projection");
  assert.deepEqual(output.findings, [finding]);
});

test("recovery_command_labels_raw_and_legacy_findings_sources", (context) => {
  const projectedFinding = { id: "F-504", round: 2, reifiedBy: "projected output source", reified: true, state: "open", resolution: "projected --> payload" };
  const rawFinding = { id: "F-504", round: 2, reifiedBy: "raw output source", reified: true, state: "open", resolution: "raw --> payload" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [], findings: [] });
  const prefix = { ...journalComment(first, TRUST), id: 1540, created_at: "2026-08-03T00:01:00Z" };
  const fields = { ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [projectedFinding.id], findings: [projectedFinding] };
  const legacy = makeJournalRecord(fields);
  const raw = makeJournalRecord({ ...fields, ledgerFindings: [rawFinding] });
  const candidate = (id, record) => ({ id, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(record)} -->` });
  const legacyCommand = runRecoveryCommand(context, [prefix, candidate(1541, legacy)], projectedFinding, 1541);
  const rawCommand = runRecoveryCommand(context, [prefix, candidate(1542, raw)], rawFinding, 1542);

  assert.equal(legacyCommand.status, 0, legacyCommand.stderr);
  assert.equal(rawCommand.status, 0, rawCommand.stderr);
  const legacyOutput = JSON.parse(legacyCommand.stdout);
  const rawOutput = JSON.parse(rawCommand.stdout);
  assert.deepEqual(
    { findingsSource: legacyOutput.findingsSource, findings: legacyOutput.findings },
    { findingsSource: "legacy evaluator projection", findings: [projectedFinding] },
  );
  assert.deepEqual(
    { findingsSource: rawOutput.findingsSource, findings: rawOutput.findings },
    { findingsSource: "raw pre-projection ledger snapshot", findings: [rawFinding] },
  );
  assert.deepEqual(legacyOutput.projectedFindings, [projectedFinding]);
  assert.deepEqual(rawOutput.projectedFindings, [projectedFinding]);
  assert.notDeepEqual(rawOutput.findings, rawOutput.projectedFindings);
  assert.equal(legacyOutput.unauthenticatedAnchor, null);
  for (const runbookPath of [".github/scripts/README.md", "docs/review-stalls.md"]) {
    const runbook = fs.readFileSync(runbookPath, "utf8");
    const outputClaim = paragraphContaining(
      runbook,
      /reports the candidate's `unauthenticatedAnchor`/i,
      `${runbookPath} recovery output contract`,
    );
    assert.match(outputClaim, /null.*legacy candidate/is);
  }
});

test("recovery_command_prefers_nested_app_attribution_and_rejects_conflicting_top_level_values", (context) => {
  const finding = { id: "F-212", round: 2, reifiedBy: "nested attribution", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [], findings: [] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [finding.id], findings: [finding] });
  const nested = { slug: TRUST.appSlug, id: TRUST.appId };
  const consistent = [
    { ...journalComment(first, TRUST), id: 1201, created_at: "2026-08-03T00:01:00Z", performed_via_github_app: nested },
    { id: 1202, app_slug: TRUST.appSlug, app_id: TRUST.appId, performed_via_github_app: nested, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(second)} -->` },
  ];
  const conflicting = consistent.map((comment) => ({ ...comment, performed_via_github_app: { slug: "different-app", id: 999 } }));

  const accepted = runRecoveryCommand(context, consistent, finding, 1202);
  const refused = runRecoveryCommand(context, conflicting, finding, 1202);

  assert.equal(accepted.status, 0, accepted.stderr);
  assert.equal(refused.status, 1);
  assert.match(refused.stderr, /conflicting.*App attribution/i);
});

test("recovery_command_rejects_nonchronological_comments_and_accepts_ordered_input", (context) => {
  const finding = { id: "F-212", round: 2, reifiedBy: "chronological recovery input", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [], findings: [] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [finding.id], findings: [finding] });
  const prefix = { ...journalComment(first, TRUST), id: 1211, created_at: "2026-08-03T00:01:00Z" };
  const candidate = { id: 1212, app_slug: TRUST.appSlug, app_id: TRUST.appId, created_at: "2026-08-03T00:02:00Z", body: `${JOURNAL_MARKER}${JSON.stringify(second)} -->` };
  const unrelatedLater = { id: 1213, created_at: "2026-08-03T00:03:00Z", body: "ordinary issue comment" };

  const ordered = runRecoveryCommand(context, [prefix, candidate, unrelatedLater], finding, 1212);
  const nonchronological = runRecoveryCommand(context, [prefix, unrelatedLater, candidate], finding, 1212);

  assert.equal(ordered.status, 0, ordered.stderr);
  assert.equal(nonchronological.status, 1);
  assert.match(nonchronological.stderr, /chronological.*order/i);
});

test("same_second_disposition_authorization_boundary_is_documented", () => {
  const scriptReadme = fs.readFileSync(".github/scripts/README.md", "utf8");
  const stalls = fs.readFileSync("docs/review-stalls.md", "utf8");

  for (const claim of [scriptReadme, stalls]) {
    const boundary = paragraphContaining(
      claim,
      /same\s+second/i,
      "the same-second authorization boundary",
    );
    assert.match(boundary, /strictly\s+after/i);
    assert.match(boundary, /reissue/i);
  }
});

test("same_second_disposition_authorization_is_rejected_but_later_second_is_accepted", () => {
  const open = { id: "F-211", reified: true, state: "open" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [open.id], findings: [open] });
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-211", state: "dismissed", reason: "maintainer disposition" })} -->`;
  const finding = { id: "F-211", reified: true, state: "dismissed", evidence: { referenceId: 211, author: "maintainer", bodyDigest: sha256(body) } };
  const reference = { id: 211, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body };
  const evaluateAt = (created_at) => trusted({
    journalComments: [{ ...journalComment(record, TRUST), created_at: "2026-08-03T00:01:00Z" }],
    findings: [finding],
    references: [{ ...reference, created_at }],
  });

  const sameSecond = evaluateAt("2026-08-03T00:01:00Z");
  const laterSecond = evaluateAt("2026-08-03T00:01:01Z");

  assert.equal(sameSecond.projectedFindings[0].state, "open");
  assert.match(sameSecond.projectedFindings[0].disposalError, /not issued after/i);
  assert.equal(laterSecond.projectedFindings[0].state, "dismissed", laterSecond.message);
});

test("nonterminal_raw_comment_terminator_has_no_deletion_recovery", () => {
  const finding = { id: "F-908", reified: true, state: "open", resolution: "literal --> remains journal data" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [finding.id], findings: [finding] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "ccc", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [finding.id], findings: [finding] });
  const unsafe = { id: 1, app_slug: TRUST.appSlug, app_id: TRUST.appId, body: `${JOURNAL_MARKER}${JSON.stringify(first)} -->` };
  const successor = { ...journalComment(second, TRUST), id: 2 };

  assert.equal(trusted({ journalComments: [unsafe, successor], findings: [finding] }).state, "history-unverifiable");
  assert.equal(trusted({ journalComments: [successor], findings: [finding] }).state, "history-unverifiable");
});

test("limitation_F002_terminal_truncation_undetected", () => {
  const extendedJournal = journalFixture();
  const genuinelyTwoRecordJournal = journalFixture(2);
  const truncated = trusted({ journalComments: extendedJournal.slice(0, 2) });
  const prefixNeverExtended = trusted({ journalComments: genuinelyTwoRecordJournal });
  assert.equal(truncated.state, "converged");
  assert.deepEqual(truncated.records.map((record) => record.stateHash), ["one", "two"]);
  assert.deepEqual(truncated.records, prefixNeverExtended.records);
  assert.doesNotMatch(JSON.stringify(truncated.records), /three/);
});

test("terminal_truncation_cannot_erase_reification_before_advisory_reclassification", () => {
  const comments = journalFixture();
  const prior = { id: "F-014", reified: true, state: "open" };
  const record = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 1, priorDigest: journalRecord(comments[1]).digest, findingIds: ["F-014"], findings: [prior] });
  comments[2] = journalComment(record, TRUST);
  const truncated = trusted({ journalComments: comments.slice(0, 2), findings: [{ id: "F-014", reified: false, state: "advisory" }] });
  assert.equal(truncated.state, "converged");
  assert.equal(truncated.projectedFindings[0].state, "advisory");
});

// F-023. An authorized tombstone retires an ID permanently. Once a later record omits it, the ID
// cannot return as advisory or in any other classification.
test("tombstoned_reified_finding_id_cannot_be_reused", () => {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 1, findingIds: ["F-900"], findings: [{ id: "F-900", reified: true, state: "open" }] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 0, priorDigest: second.digest });
  const comments = [first, second, third].map((record) => journalComment(record, TRUST));
  const result = trusted({ journalComments: comments, findings: [{ id: "F-900", reified: false, state: "advisory" }] });

  assert.equal(result.ok, false);
  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /retired finding IDs cannot be reused/i);
});

test("an_active_advisory_id_is_not_mistaken_for_retired_reuse", () => {
  const advisory = { id: "F-904", reified: false, state: "advisory" };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: ["F-904"], findings: [advisory] });
  const result = trusted({ journalComments: [journalComment(record, TRUST)], findings: [advisory] });

  assert.equal(result.ok, true);
  assert.equal(result.state, "converged");
});

test("tombstoned_advisory_id_cannot_be_reused", () => {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: ["F-902"], findings: [{ id: "F-902", reified: false, state: "advisory" }] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: [], findings: [] });
  const tombstoneBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-902", state: "tombstone", reason: "retire advisory ID" })} -->`;
  const tombstoneReference = { id: 902, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body: tombstoneBody };
  const tombstone = { id: "F-902", evidence: { referenceId: 902, author: "maintainer", bodyDigest: sha256(tombstoneBody) } };
  const result = trusted({
    journalComments: [first, second].map((record) => journalComment(record, TRUST)),
    findings: [{ id: "F-902", reified: false, state: "advisory" }],
    tombstones: [tombstone],
    references: [tombstoneReference],
  });

  assert.equal(result.ok, false);
  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /retired finding IDs cannot be reused/i);
});

// F-027. A matching unkeyed digest catches incidental byte edits; it does not prove the record is
// readable. A record whose findings payload is digest-consistent but malformed used to be skipped, which silently
// discarded the reification history and reopened the F-023 path underneath the fix for it.
test("malformed_surviving_findings_record_is_history_unverifiable", () => {
  for (const malformed of ["malformed", {}, 42, undefined]) {
    const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 1, findingIds: ["F-900"], findings: malformed });
    const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest });
    const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 0, priorDigest: second.digest });
    const comments = [first, second, third].map((record) => journalComment(record, TRUST));
    const result = trusted({ journalComments: comments, findings: [{ id: "F-900", reified: false, state: "advisory" }] });

    assert.equal(result.state, "history-unverifiable", `findings: ${JSON.stringify(malformed)} must fail closed`);
  }
});

// The two lists must agree, so a record cannot claim an ID while hiding what that finding was.
test("journal_record_whose_lists_disagree_is_history_unverifiable", () => {
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 1, findingIds: ["F-900"], findings: [{ id: "F-901", reified: true, state: "open" }] });
  const comments = [first].map((record) => journalComment(record, TRUST));

  assert.equal(trusted({ journalComments: comments }).state, "history-unverifiable");
});

// The same union lookup must not manufacture a blocker out of an ordinary advisory finding
// that no surviving record ever carried as reified.
test("advisory_finding_never_reified_in_any_record_still_converges", () => {
  const result = trusted({ findings: [{ id: "F-901", reified: false, state: "advisory" }] });

  assert.equal(result.state, "converged");
  assert.equal(result.projectedFindings[0].state, "advisory");
});

test("trusted_evaluator_rejects_parseFindings_error_instead_of_converging", () => {
  assert.throws(
    () => requireParsedFindings({ error: "Review ledger: duplicate F-001." }),
    /duplicate F-001/,
  );
  assert.deepEqual(requireParsedFindings({ findings: [] }), []);
});

test("trusted_evaluator_rejects_open_finding_that_names_no_failing_check", () => {
  const result = trusted({ findings: [{ id: "F-015", state: "open", reified: false }] });
  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /open.*names no failing check/i);
});

test("trusted_evaluator_rejects_reified_advisory_state", () => {
  const result = trusted({ findings: [{ id: "F-016", state: "advisory", reified: true }] });
  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /reified.*advisory/i);
});

test("verified disposal requires exact directive, independent authorized author and body digest", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-013", state: "dismissed", reason: "ADR 0008" })} -->`;
  const reference = { id: 91, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body };
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

test("edited_authorization_cannot_regain_force_by_updating_head_digest", () => {
  const originalBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-903", state: "dismissed", reason: "original decision" })} -->`;
  const originalEvidence = { referenceId: 903, author: "maintainer", bodyDigest: sha256(originalBody) };
  const recordedFinding = { id: "F-903", reified: true, state: "dismissed", evidence: originalEvidence };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: ["F-903"], findings: [recordedFinding] });
  const editedBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-903", state: "dismissed", reason: "edited decision" })} -->`;
  const editedEvidence = { ...originalEvidence, bodyDigest: sha256(editedBody) };
  const reference = { id: 903, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body: editedBody };
  const result = trusted({
    journalComments: [journalComment(record, TRUST)],
    findings: [{ ...recordedFinding, evidence: editedEvidence }],
    references: [reference],
  });

  assert.equal(result.ok, false);
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /body digest does not match/i);
});

test("reopened_finding_rejects_stale_authorization_for_prior_disposition", () => {
  const originalBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-905", state: "dismissed", reason: "original priority decision" })} -->`;
  const originalEvidence = { referenceId: 905, author: "maintainer", bodyDigest: sha256(originalBody) };
  const dismissed = { id: "F-905", reified: true, state: "dismissed", evidence: originalEvidence };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: ["F-905"], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: ["F-905"], findings: [{ id: "F-905", reified: true, state: "open" }] });
  const comments = [
    { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
    { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
  ];
  const reference = { id: 905, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:00:00Z", body: originalBody };
  const result = trusted({
    journalComments: comments,
    findings: [{ ...dismissed, evidence: originalEvidence }],
    references: [reference],
  });

  assert.equal(result.ok, false);
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /after the finding was reopened/i);
});

test("reopened_finding_accepts_fresh_authorization_for_new_disposition", () => {
  const dismissedBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-905", state: "dismissed", reason: "original priority decision" })} -->`;
  const dismissed = { id: "F-905", reified: true, state: "dismissed", evidence: { referenceId: 905, author: "maintainer", bodyDigest: sha256(dismissedBody) } };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: ["F-905"], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: ["F-905"], findings: [{ id: "F-905", reified: true, state: "open" }] });
  const resolvedBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-905", state: "resolved", reason: "reopened defect now passes" })} -->`;
  const resolved = { id: "F-905", reified: true, state: "resolved", evidence: { referenceId: 906, author: "maintainer", bodyDigest: sha256(resolvedBody), checkName: "Check, Test & Lint" } };
  const reference = { id: 906, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:03:00Z", body: resolvedBody };
  const check = { id: 6, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
    ],
    findings: [resolved], references: [reference], checks: [check],
  });

  assert.equal(result.ok, true, result.message);
  assert.equal(result.state, "converged");
  assert.equal(result.projectedFindings[0].state, "resolved");
});

test("stale_directive_cannot_return_after_intervening_non_open_disposition", () => {
  const dismissedBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-909", state: "dismissed", reason: "original priority decision" })} -->`;
  const dismissedEvidence = { referenceId: 909, author: "maintainer", bodyDigest: sha256(dismissedBody) };
  const dismissed = { id: "F-909", reified: true, state: "dismissed", evidence: dismissedEvidence };
  const resolved = { id: "F-909", reified: true, state: "resolved", evidence: { referenceId: 910, author: "maintainer", bodyDigest: "recorded-resolution" } };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [dismissed.id], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: [resolved.id], findings: [resolved] });
  const reference = { id: 909, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:00:00Z", body: dismissedBody };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
    ],
    findings: [dismissed], references: [reference],
  });

  assert.equal(result.ok, false);
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /disposition changed/i);
});

test("historical_absence_cannot_be_collapsed_into_one_disposition_run", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-913", state: "dismissed", reason: "stale priority decision" })} -->`;
  const evidence = { referenceId: 920, author: "maintainer", bodyDigest: sha256(body) };
  const dismissed = { id: "F-913", reified: true, state: "dismissed", evidence };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [dismissed.id], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: [], findings: [] });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 0, priorDigest: second.digest, findingIds: [dismissed.id], findings: [dismissed] });
  const reference = { id: 920, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:00:00Z", body };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
      { ...journalComment(third, TRUST), created_at: "2026-08-03T00:03:00Z" },
    ],
    findings: [dismissed], references: [reference],
  });

  assert.equal(result.ok, false);
  assert.equal(result.state, "history-unverifiable");
  assert.match(result.message, /retired finding IDs cannot be reused/i);
});

test("recorded_disposition_context_treats_absence_as_a_freshness_boundary", () => {
  const dismissed = { id: "F-210", reified: true, state: "dismissed", evidence: { referenceId: 210 } };
  const records = [
    { commentCreatedAt: "2026-08-03T00:01:00Z", findings: [dismissed] },
    { commentCreatedAt: "2026-08-03T00:02:00Z", findings: [] },
    { commentCreatedAt: "2026-08-03T00:03:00Z", findings: [dismissed] },
  ];
  const continuous = [records[0], { commentCreatedAt: "2026-08-03T00:02:00Z", findings: [dismissed] }, records[2]];

  assert.equal(recordedDispositionContext(records, dismissed.id, dismissed.state).changedAt, "2026-08-03T00:02:00Z");
  assert.equal(recordedDispositionContext(continuous, dismissed.id, dismissed.state).changedAt, null);
});

test("continuous_presence_is_not_mistaken_for_an_interrupted_disposition_run", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-914", state: "dismissed", reason: "continuous priority decision" })} -->`;
  const evidence = { referenceId: 921, author: "maintainer", bodyDigest: sha256(body) };
  const dismissed = { id: "F-914", reified: true, state: "dismissed", evidence };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [dismissed.id], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: [dismissed.id], findings: [dismissed] });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 0, priorDigest: second.digest, findingIds: [dismissed.id], findings: [dismissed] });
  const reference = { id: 921, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:00:00Z", body };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
      { ...journalComment(third, TRUST), created_at: "2026-08-03T00:03:00Z" },
    ],
    findings: [dismissed], references: [reference],
  });

  assert.equal(result.ok, true, result.message);
  assert.equal(result.projectedFindings[0].state, "dismissed");
});

test("fresh_directive_after_intervening_non_open_disposition_is_accepted", () => {
  const originalBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-909", state: "dismissed", reason: "original priority decision" })} -->`;
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: ["F-909"], findings: [{ id: "F-909", reified: true, state: "dismissed", evidence: { referenceId: 909, author: "maintainer", bodyDigest: sha256(originalBody) } }] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: ["F-909"], findings: [{ id: "F-909", reified: true, state: "resolved", evidence: { referenceId: 910, author: "maintainer", bodyDigest: "recorded-resolution" } }] });
  const freshBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-909", state: "dismissed", reason: "new priority decision" })} -->`;
  const freshEvidence = { referenceId: 911, author: "maintainer", bodyDigest: sha256(freshBody) };
  const reference = { id: 911, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:03:00Z", body: freshBody };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
    ],
    findings: [{ id: "F-909", reified: true, state: "dismissed", evidence: freshEvidence }], references: [reference],
  });

  assert.equal(result.ok, true, result.message);
  assert.equal(result.projectedFindings[0].state, "dismissed");
});

test("recorded_stale_return_after_intervening_disposition_is_rejected", () => {
  const staleBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-911", state: "dismissed", reason: "original priority decision" })} -->`;
  const staleEvidence = { referenceId: 915, author: "maintainer", bodyDigest: sha256(staleBody) };
  const dismissed = { id: "F-911", reified: true, state: "dismissed", evidence: staleEvidence };
  const resolved = { id: "F-911", reified: true, state: "resolved", evidence: { referenceId: 916, author: "maintainer", bodyDigest: "recorded-resolution" } };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [dismissed.id], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: [resolved.id], findings: [resolved] });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 0, priorDigest: second.digest, findingIds: [dismissed.id], findings: [dismissed] });
  const reference = { id: 915, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:00:00Z", body: staleBody };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
      { ...journalComment(third, TRUST), created_at: "2026-08-03T00:03:00Z" },
    ],
    findings: [dismissed], references: [reference],
  });

  assert.equal(result.ok, false);
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /disposition changed/i);
});

test("recorded_fresh_return_after_intervening_disposition_is_accepted", () => {
  const originalBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-912", state: "dismissed", reason: "original priority decision" })} -->`;
  const resolved = { id: "F-912", reified: true, state: "resolved", evidence: { referenceId: 918, author: "maintainer", bodyDigest: "recorded-resolution" } };
  const freshBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-912", state: "dismissed", reason: "renewed priority decision" })} -->`;
  const freshEvidence = { referenceId: 919, author: "maintainer", bodyDigest: sha256(freshBody) };
  const returned = { id: "F-912", reified: true, state: "dismissed", evidence: freshEvidence };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [returned.id], findings: [{ ...returned, evidence: { referenceId: 917, author: "maintainer", bodyDigest: sha256(originalBody) } }] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 0, priorDigest: first.digest, findingIds: [resolved.id], findings: [resolved] });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 0, priorDigest: second.digest, findingIds: [returned.id], findings: [returned] });
  const reference = { id: 919, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:02:30Z", body: freshBody };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
      { ...journalComment(third, TRUST), created_at: "2026-08-03T00:03:00Z" },
    ],
    findings: [returned], references: [reference],
  });

  assert.equal(result.ok, true, result.message);
  assert.equal(result.projectedFindings[0].state, "dismissed");
});

test("fresh_directive_after_reopening_survives_later_open_observations", () => {
  const dismissed = { id: "F-910", reified: true, state: "dismissed", evidence: { referenceId: 912, author: "maintainer", bodyDigest: "recorded-dismissal" } };
  const open = { id: "F-910", reified: true, state: "open" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [dismissed.id], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [open.id], findings: [open] });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 1, priorDigest: second.digest, findingIds: [open.id], findings: [open] });
  const freshBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-910", state: "dismissed", reason: "fresh decision after reopening" })} -->`;
  const freshEvidence = { referenceId: 913, author: "maintainer", bodyDigest: sha256(freshBody) };
  const reference = { id: 913, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:03:00Z", body: freshBody };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
      { ...journalComment(third, TRUST), created_at: "2026-08-03T00:04:00Z" },
    ],
    findings: [{ id: "F-910", reified: true, state: "dismissed", evidence: freshEvidence }], references: [reference],
  });

  assert.equal(result.ok, true, result.message);
  assert.equal(result.projectedFindings[0].state, "dismissed");
});

test("directive_before_first_reopening_remains_stale_after_later_open_observations", () => {
  const dismissed = { id: "F-910", reified: true, state: "dismissed", evidence: { referenceId: 912, author: "maintainer", bodyDigest: "recorded-dismissal" } };
  const open = { id: "F-910", reified: true, state: "open" };
  const first = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0, findingIds: [dismissed.id], findings: [dismissed] });
  const second = makeJournalRecord({ ...TRUST, observation: 2, headSha: "bbb", stateHash: "two", blocking: 1, priorDigest: first.digest, findingIds: [open.id], findings: [open] });
  const third = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 1, priorDigest: second.digest, findingIds: [open.id], findings: [open] });
  const staleBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-910", state: "dismissed", reason: "decision issued before reopening" })} -->`;
  const staleEvidence = { referenceId: 914, author: "maintainer", bodyDigest: sha256(staleBody) };
  const reference = { id: 914, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", created_at: "2026-08-03T00:01:30Z", body: staleBody };
  const result = trusted({
    journalComments: [
      { ...journalComment(first, TRUST), created_at: "2026-08-03T00:01:00Z" },
      { ...journalComment(second, TRUST), created_at: "2026-08-03T00:02:00Z" },
      { ...journalComment(third, TRUST), created_at: "2026-08-03T00:04:00Z" },
    ],
    findings: [{ id: "F-910", reified: true, state: "dismissed", evidence: staleEvidence }], references: [reference],
  });

  assert.equal(result.ok, false);
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /after the finding was reopened/i);
});

test("authorization_comment_with_second_malformed_directive_is_refused", () => {
  const valid = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-906", state: "dismissed", reason: "authorized" })} -->`;
  const body = `${valid}\n${DIRECTIVE_MARKER}{not-json} -->`;
  const finding = { id: "F-906", reified: true, state: "dismissed", evidence: { referenceId: 906, author: "maintainer", bodyDigest: sha256(body) } };
  const reference = { id: 906, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body };
  const result = trusted({ findings: [finding], references: [reference] });

  assert.equal(result.ok, false);
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /machine-readable directive/i);
});

test("authorization_comment_with_multiple_valid_directives_is_accepted", () => {
  const matching = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-906", state: "dismissed", reason: "authorized" })} -->`;
  const unrelated = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-907", state: "dismissed", reason: "separate finding" })} -->`;
  const body = `${matching}\n${unrelated}`;
  const finding = { id: "F-906", reified: true, state: "dismissed", evidence: { referenceId: 906, author: "maintainer", bodyDigest: sha256(body) } };
  const reference = { id: 906, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body };

  assert.equal(trusted({ findings: [finding], references: [reference] }).state, "converged");
});

test("self_authorization_refuses_a_second_principal_with_owner_present_or_absent", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-930", state: "dismissed", reason: "maintainer decision" })} -->`;
  const finding = { id: "F-930", reified: true, state: "dismissed", evidence: { referenceId: 930, author: "maintainer", bodyDigest: sha256(body) } };
  const reference = { id: 930, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  for (const principals of [["maintainer", "second-principal"], ["second-principal"]]) {
    const result = trusted({ pull: { number: 200, author: "maintainer", headSha: "ccc", headRepositoryId: 7, baseRepositoryId: 7 }, findings: [finding], references: [reference], principalEnumeration: { ok: true, principals } });
    assert.equal(result.projectedFindings[0].state, "open");
    assert.match(result.projectedFindings[0].disposalError, /another repository principal/i);
  }
});

test("pull_request_author_directive_disposes_and_is_recorded_on_first_evaluation", async () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-929", state: "dismissed", reason: "accepted priority decision" })} -->`;
  const evidence = { referenceId: 929, author: "maintainer", bodyDigest: sha256(body) };
  const finding = { id: "F-929", reified: true, state: "dismissed", evidence };
  const reference = { id: 929, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const result = trusted({ pull: { number: 200, author: "maintainer", headSha: "ccc", headRepositoryId: 7, baseRepositoryId: 7 }, findings: [finding], references: [reference] });
  assert.equal(result.state, "converged");
  assert.deepEqual(result.projectedFindings[0].evidence, evidence);

  let persisted;
  await publishEvaluation({
    result,
    appendJournal: async () => {
      persisted = makeJournalRecord({ ...TRUST, observation: 4, priorDigest: result.records[2].digest, headSha: "ccc", findingIds: [finding.id], findings: result.projectedFindings, stateHash: result.currentHash, blocking: result.blocking });
    },
    reportCheck: async () => {}, setFailed: () => {}, info: () => {},
  });
  assert.deepEqual(persisted.findings[0].evidence, evidence);
});

test("contract constants are pinned", () => {
  assert.deepStrictEqual([...STATES], ["open", "resolved", "dismissed", "advisory"]);
  assert.deepStrictEqual([...FAILING_CONCLUSIONS], ["failure", "timed_out", "cancelled", "action_required", "stale"]);
  assert.deepStrictEqual(AUTHORIZING_TRANSITIONS, [
    { state: "resolved", path: "finding" },
    { state: "dismissed", path: "finding" },
    { state: "advisory", path: "finding" },
    { state: "tombstone", path: "special" },
    { state: "genesis", path: "special" },
  ]);
  assert.deepStrictEqual([...AUTHORIZING_ASSOCIATIONS], ["MEMBER", "OWNER", "COLLABORATOR"]);
  assert.deepStrictEqual(REQUIRED_JOBS, ["Check, Test & Lint", "Knowledge graph up to date"]);
});

test("unknown_principal_enumeration_refuses_the_authorizing_transition_product", () => {
  const principalEnumeration = { ok: false, reason: "enumeration unavailable" };
  const explicitPull = { number: 200, author: "maintainer", headSha: "ccc", headRepositoryId: 7, baseRepositoryId: 7 };
  const directive = ({ id, state, referenceId, author, association, kind }) => {
    const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: id, state, reason: `authorize ${state}` })} -->`;
    const evidence = { referenceId, author, bodyDigest: sha256(body) };
    const reference = { id: referenceId, kind, state: kind === "review" ? "APPROVED" : undefined, repositoryId: 7, pullNumber: 200, user: { login: author }, author_association: association, body };
    return { evidence, reference };
  };
  const validResolutionCheck = { id: 31, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };
  const ids = { resolved: "F-931", dismissed: "F-932", advisory: "F-933", tombstone: "F-934", genesis: "GENESIS" };
  const referenceIds = { resolved: 931, dismissed: 932, advisory: 933, tombstone: 934, genesis: 935 };

  for (const transition of AUTHORIZING_TRANSITIONS) {
    for (const author of [explicitPull.author, "second-principal"]) {
      for (const association of AUTHORIZING_ASSOCIATIONS) {
        for (const kind of ["comment", "review"]) {
          const authorization = directive({ id: ids[transition.state], state: transition.state, referenceId: referenceIds[transition.state], author, association, kind });
          let result;
          let rejection;
          if (transition.state === "resolved") {
            result = trusted({ pull: explicitPull, findings: [{ id: ids.resolved, reified: true, state: "resolved", evidence: { ...authorization.evidence, checkName: "Check, Test & Lint" } }], references: [authorization.reference], checks: [validResolutionCheck], principalEnumeration });
            rejection = result.projectedFindings[0];
          } else if (transition.state === "dismissed") {
            result = trusted({ pull: explicitPull, findings: [{ id: ids.dismissed, reified: true, state: "dismissed", evidence: authorization.evidence }], references: [authorization.reference], principalEnumeration });
            rejection = result.projectedFindings[0];
          } else if (transition.state === "advisory") {
            authorization.reference.created_at = "2026-08-03T00:02:00Z";
            const prior = { id: ids.advisory, reified: true, state: "open" };
            const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: "one", blocking: 1, findingIds: [prior.id], findings: [prior] });
            result = trusted({ pull: explicitPull, journalComments: [{ ...journalComment(record, TRUST), created_at: "2026-08-03T00:01:00Z" }], findings: [{ id: ids.advisory, reified: false, state: "advisory", evidence: authorization.evidence }], references: [authorization.reference], principalEnumeration });
            rejection = result.projectedFindings[0];
          } else if (transition.state === "tombstone") {
            result = trusted({ pull: explicitPull, tombstones: [{ id: ids.tombstone, evidence: authorization.evidence }], references: [authorization.reference], principalEnumeration });
            rejection = { state: result.state, disposalError: result.message };
          } else {
            result = trusted({ pull: explicitPull, journalComments: [], genesisEvidence: authorization.evidence, references: [authorization.reference], principalEnumeration });
            rejection = result.syntheticFindings[0] || {};
          }
          const cell = `${transition.state}/${author === explicitPull.author ? "self" : "third-party"}/${association}/${kind}`;
          assert.equal(rejection.state, transition.path === "special" && transition.state === "tombstone" ? "history-unverifiable" : "open", `${cell} must be rejected`);
          assert.match(rejection.disposalError, /enumeration is unknown/i, `${cell} must fail on unknown enumeration`);
        }
      }
    }
  }
});

test("authorization_refuses_a_target_state_outside_the_transition_registry", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-936", state: "closed", reason: "completed outside the contract" })} -->`;
  const finding = { id: "F-936", reified: true, state: "closed", evidence: { referenceId: 936, author: "maintainer", bodyDigest: sha256(body) } };
  const reference = { id: 936, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };

  const result = verifyAuthorization({ finding, reference, pullAuthor: "different-author", targetState: "closed", repositoryId: 7, pullNumber: 200, maintainerLogin: "maintainer", principalEnumeration: COMPLETE_PRINCIPAL_ENUMERATION });

  assert.equal(result.ok, false);
  assert.match(result.reason, /not an authorizing transition/i);
});

test("repository_principal_enumeration_follows_next_links_and_fails_closed", async () => {
  const requests = [];
  const complete = await enumerateRepositoryPrincipals({
    owner: "jsunyermias", repo: "keeplin",
    request: async (url) => {
      requests.push(url);
      if (requests.length === 1) return { status: 200, data: [{ login: "maintainer" }], headers: { link: '<https://api.github.test/page/2>; rel="next"' } };
      return { status: 200, data: [{ login: "second-principal" }], headers: {} };
    },
  });
  assert.deepEqual(complete, { ok: true, principals: ["maintainer", "second-principal"] });
  assert.equal(requests.length, 2);

  for (const request of [
    async () => { const error = new Error("forbidden"); error.status = 403; throw error; },
    async () => { const error = new Error("rate limited"); error.status = 429; throw error; },
    async () => { throw new Error("transport"); },
    async () => ({ status: 200, data: [], headers: { link: 'not-a-url; rel="next"' } }),
  ]) assert.equal((await enumerateRepositoryPrincipals({ request, owner: "jsunyermias", repo: "keeplin" })).ok, false);
});

test("evaluator_GITHUB_TOKEN_really_enumerates_the_expected_repository_principals", { skip: process.env.CI !== "true" || !process.env.GITHUB_TOKEN }, async () => {
  const repository = process.env.GITHUB_REPOSITORY || "jsunyermias/keeplin";
  const [owner, repo] = repository.split("/");
  const result = await enumerateRepositoryPrincipals({
    owner, repo,
    request: async (url) => {
      const response = await fetch(url, { headers: { accept: "application/vnd.github+json", authorization: `Bearer ${process.env.GITHUB_TOKEN}`, "x-github-api-version": "2022-11-28" } });
      return { status: response.status, data: await response.json(), headers: { link: response.headers.get("link") } };
    },
  });
  assert.deepEqual(result, { ok: true, principals: ["jsunyermias"] });
});

test("disposal_reference_from_another_pull_request_or_repository_is_refused", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-013", state: "dismissed", reason: "authorized" })} -->`;
  const finding = { id: "F-013", reified: true, state: "dismissed", evidence: { referenceId: 91, author: "maintainer", bodyDigest: sha256(body) } };
  const reference = { id: 91, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body };
  for (const mutation of [{ repositoryId: 8 }, { pullNumber: 201 }]) {
    const result = verifyAuthorization({ finding, reference: { ...reference, ...mutation }, pullAuthor: "author", targetState: "dismissed", repositoryId: 7, pullNumber: 200, maintainerLogin: "maintainer", principalEnumeration: COMPLETE_PRINCIPAL_ENUMERATION });
    assert.equal(result.ok, false);
    assert.match(result.reason, /repository and pull request/);
  }
});

test("prior_reified_finding_cannot_be_reclassified_advisory_without_verified_authorization", () => {
  const comments = journalFixture();
  const prior = { id: "F-014", reified: true, state: "open" };
  const record = makeJournalRecord({ ...TRUST, observation: 3, headSha: "ccc", stateHash: "three", blocking: 1, priorDigest: journalRecord(comments[1]).digest, findingIds: ["F-014"], findings: [prior] });
  comments[2] = journalComment(record, TRUST);
  const result = trusted({ journalComments: comments, findings: [{ id: "F-014", reified: false, state: "advisory" }] });
  assert.equal(result.state, "converging");
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /authorization/);
});

test("resolved evidence derives one successful required check from the evaluated run", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-013", state: "resolved", reason: "test passes" })} -->`;
  const reference = { id: 92, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const finding = { id: "F-013", reified: true, state: "resolved", evidence: { referenceId: 92, author: "maintainer", bodyDigest: sha256(body), checkName: "Check, Test & Lint" } };
  const check = { id: 5, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };
  assert.equal(trusted({ findings: [finding], references: [reference], checks: [check] }).state, "converged");
  for (const mutation of [{ status: "in_progress" }, { head_sha: "old" }, { workflow_id: 99 }, { workflow_run_id: 66 }, { app_slug: "other-app" }, { app_id: 1 }, { conclusion: "neutral" }]) {
    assert.equal(trusted({ findings: [finding], references: [reference], checks: [{ ...check, ...mutation }] }).projectedFindings[0].state, "open");
  }
});

test("resolved evidence fails explicitly when the evaluated run has zero or multiple named checks", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-016", state: "resolved", reason: "test passes" })} -->`;
  const finding = { id: "F-016", reified: true, state: "resolved", evidence: { referenceId: 95, author: "maintainer", bodyDigest: sha256(body), checkName: "Check, Test & Lint" } };
  const reference = { id: 95, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const check = { id: 15, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };

  const absent = trusted({ findings: [finding], references: [reference], checks: [] });
  assert.equal(absent.projectedFindings[0].state, "open");
  assert.match(absent.projectedFindings[0].disposalError, /absent from the evaluated workflow run/i);

  const duplicate = trusted({ findings: [finding], references: [reference], checks: [check, { ...check, id: 16 }] });
  assert.equal(duplicate.projectedFindings[0].state, "open");
  assert.match(duplicate.projectedFindings[0].disposalError, /ambiguous within the evaluated workflow run/i);
});

test("resolved evidence rejects a successful non-required check from the evaluated run", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-018", state: "resolved", reason: "test passes" })} -->`;
  const finding = { id: "F-018", reified: true, state: "resolved", evidence: { referenceId: 97, author: "maintainer", bodyDigest: sha256(body), checkName: "Untrusted Helper" } };
  const reference = { id: 97, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const check = { id: 18, name: "Untrusted Helper", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };

  const result = trusted({ findings: [finding], references: [reference], checks: [check] });
  assert.notEqual(result.state, "converged");
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /not an explicitly required job/i);
});

test("legacy checkRunId is ignored while the evaluator derives current-run proof", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-017", state: "resolved", reason: "test passes" })} -->`;
  const finding = { id: "F-017", reified: true, state: "resolved", evidence: { referenceId: 96, author: "maintainer", bodyDigest: sha256(body), checkRunId: 999, checkName: "Check, Test & Lint" } };
  const reference = { id: 96, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const check = { id: 17, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };
  assert.equal(trusted({ findings: [finding], references: [reference], checks: [check] }).state, "converged");
});

test("unchanged_resolved_disposition_uses_current_run_resolution_proof", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-913", state: "resolved", reason: "test passes" })} -->`;
  const authorization = { referenceId: 920, author: "maintainer", bodyDigest: sha256(body) };
  const recorded = { id: "F-913", reified: true, state: "resolved", evidence: { ...authorization, checkRunId: 7, checkName: "Check, Test & Lint" } };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 0, findingIds: [recorded.id], findings: [recorded] });
  const current = { ...recorded, evidence: { ...authorization, checkName: "Check, Test & Lint" } };
  const reference = { id: 920, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const priorCheck = { id: 7, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };
  const currentCheck = { ...priorCheck, id: 8, workflow_run_id: 78 };
  const result = trusted({
    journalComments: [journalComment(record, TRUST)], findings: [current], references: [reference],
    checks: [priorCheck, currentCheck], config: { ...TRUST, runId: 78 },
  });

  assert.equal(result.ok, true, result.message);
  assert.equal(result.projectedFindings[0].state, "resolved");
  assert.equal(result.projectedFindings[0].evidence.checkRunId, undefined);
});

test("unchanged_resolved_disposition_keeps_recorded_authorization_with_current_proof", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-915", state: "resolved", reason: "test passes" })} -->`;
  const authorization = { referenceId: 922, author: "maintainer", bodyDigest: sha256(body) };
  const recorded = { id: "F-915", reified: true, state: "resolved", evidence: { ...authorization, checkRunId: 11, checkName: "Check, Test & Lint" } };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 0, findingIds: [recorded.id], findings: [recorded] });
  const current = { ...recorded, evidence: { referenceId: 999, author: "replacement", bodyDigest: "replacement", checkName: "Check, Test & Lint" } };
  const reference = { id: 922, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const priorCheck = { id: 11, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 77, app_slug: "github-actions", app_id: 15368 };
  const currentCheck = { ...priorCheck, id: 12, workflow_run_id: 78 };
  const result = trusted({
    journalComments: [journalComment(record, TRUST)], findings: [current], references: [reference],
    checks: [priorCheck, currentCheck], config: { ...TRUST, runId: 78 },
  });

  assert.equal(result.ok, true, result.message);
  assert.deepEqual(result.projectedFindings[0].evidence, { ...authorization, checkName: "Check, Test & Lint" });
});

test("unchanged_resolved_disposition_rejects_current_failed_resolution_proof", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-914", state: "resolved", reason: "test passes" })} -->`;
  const authorization = { referenceId: 921, author: "maintainer", bodyDigest: sha256(body) };
  const recorded = { id: "F-914", reified: true, state: "resolved", evidence: { ...authorization, checkRunId: 9, checkName: "Check, Test & Lint" } };
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 0, findingIds: [recorded.id], findings: [recorded] });
  const current = { ...recorded, evidence: { ...authorization, checkName: "Check, Test & Lint" } };
  const reference = { id: 921, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const priorCheck = { id: 9, name: "Check, Test & Lint", status: "completed", conclusion: "success", head_sha: "ccc", workflow_id: 88, workflow_run_id: 76, app_slug: "github-actions", app_id: 15368 };
  const failedCurrentCheck = { ...priorCheck, id: 10, workflow_run_id: 77, conclusion: "failure" };
  const result = trusted({
    journalComments: [journalComment(record, TRUST)], findings: [current], references: [reference],
    checks: [priorCheck, failedCurrentCheck],
  });

  assert.equal(result.ok, false);
  assert.equal(result.projectedFindings[0].state, "open");
  assert.match(result.projectedFindings[0].disposalError, /required successful job/i);
});

test("trusted_adapter_rejects_triggering_or_check_workflow_id_not_equal_to_configured_ci_workflow_id", () => {
  const workflow = fs.readFileSync(".github/workflows/review-loop-evaluator.yml", "utf8");
  assert.match(workflow, /CI_WORKFLOW_ID:.*vars\.CI_WORKFLOW_ID/);
  assert.match(workflow, /run\.workflow_id !== configuredCiWorkflowId/);
  assert.match(workflow, /checkRun\.workflow_id !== configuredCiWorkflowId/);
  assert.doesNotMatch(workflow, /workflow_id:\s*run\.workflow_id/);
});

test("trusted_adapter_executes_normalized_check_pages_without_creating_undefined_items", async () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const pullBody = "## Review ledger\n\n| ID | Round | Reified by | State | Resolution |\n|---|---|---|---|---|\n\n### Round log\n";
  const check = { id: 17, name: "Optional check", status: "completed", conclusion: "success", head_sha: "ccc", details_url: "" };
  const prior = makeJournalRecord({ ...TRUST, runId: 76, runAttempt: 1, observation: 1, headSha: "bbb", stateHash: "prior", blocking: 0 });
  const comments = [{ ...journalComment(prior, TRUST), id: 1500, created_at: "2026-08-03T00:01:00Z", performed_via_github_app: { slug: TRUST.appSlug, id: TRUST.appId } }];
  const cases = [
    [{ data: [] }],
    [{ data: [check] }],
    [{ data: [] }, { data: [check] }, { data: [] }],
  ];

  for (const checkPages of cases) {
    await executeTrustedWorkflow(repositoryRoot, pullBody, comments, { checkPages, expectFailure: false });
  }
});

test("trusted_adapter_fails_closed_on_malformed_job_or_check_pagination_evidence", async () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const pullBody = "## Review ledger\n\n| ID | Round | Reified by | State | Resolution |\n|---|---|---|---|---|\n\n### Round log\n";
  const cases = [
    [{ checkPages: [{ data: {} }] }, /check-run evidence: malformed pagination response/i],
    [{ checkPages: [{ data: [undefined] }] }, /check-run evidence: malformed paginated item/i],
    [{ jobPages: [{ data: {} }] }, /job evidence: malformed pagination response/i],
    [{ jobPages: [{ data: [undefined] }] }, /job evidence: malformed paginated item/i],
  ];

  for (const [pagination, expected] of cases) {
    const outcome = await executeTrustedWorkflow(repositoryRoot, pullBody, journalFixture(), {
      ...pagination,
      expectJournal: false,
    });
    assert.equal(outcome.postedBody, undefined, "malformed evidence must not produce a journal observation");
    assert.equal(outcome.failures.length, 1);
    assert.match(outcome.failures[0], expected);
  }
});

test("trusted_adapter_fails_closed_on_malformed_metadata_or_check_lookup", () => {
  const workflow = fs.readFileSync(".github/workflows/review-loop-evaluator.yml", "utf8");
  assert.match(workflow, /Malformed trusted review-loop metadata/);
  assert.match(workflow, /Unable to verify workflow identity for a resolution check in the evaluated run/);
});

test("invalid_tombstone_identifier_is_refused_without_echoing_a_journal_marker", () => {
  const maliciousId = `F-999 ${JOURNAL_MARKER}{\"forged\":true} -->`;
  const result = trusted({ tombstones: [{ id: maliciousId, evidence: { referenceId: 999 } }] });

  assert.equal(result.state, "history-unverifiable");
  assert.doesNotMatch(result.message, /keeplin-review-loop-journal/);
  assert.match(result.message, /invalid tombstone identifier/i);
});

test("journal_comment_neutralizes_markers_in_messages_before_persistence", () => {
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 1, findingIds: ["F-999"], findings: [{ id: "F-999", reified: true, state: "open", resolution: `record ${JOURNAL_MARKER}7 -->` }] });
  const message = `Blocked ${JOURNAL_MARKER}{\"forged\":true} -->`;
  const body = journalComment(record, TRUST, message).body;

  assert.equal(body.split(JOURNAL_MARKER).length - 1, 1);
  assert.match(body, /Blocked/);
  assert.match(body, /forged/);
  assert.doesNotMatch(body.slice(body.indexOf(" -->") + 4), /<!-- keeplin-review-loop-/);
  const workflow = fs.readFileSync(".github/workflows/review-loop-evaluator.yml", "utf8");
  assert.match(workflow, /body: evaluator\.journalComment\(record, config, result\.message\)\.body/);
});

test("journal_comment_preserves_an_ordinary_message_verbatim", () => {
  const record = makeJournalRecord({ ...TRUST, observation: 1, headSha: "aaa", stateHash: "one", blocking: 0 });
  const message = "Review loop converged.";

  assert.ok(journalComment(record, TRUST, message).body.endsWith(`\n\n${message}`));
});

test("trusted_workflow_ignores_push_runs", () => {
  const workflow = fs.readFileSync(".github/workflows/review-loop-evaluator.yml", "utf8");
  assert.match(workflow, /run\.event !== 'pull_request'/);
  assert.match(workflow, /Only pull_request workflow runs are review rounds/);
});

test("trusted_workflow_queues_pending_runs_and_documents_the_queue_bound", () => {
  const workflow = fs.readFileSync(".github/workflows/review-loop-evaluator.yml", "utf8");
  const workflowCompanion = fs.readFileSync(".github/workflows/review-loop-evaluator.yml.md", "utf8");
  const scriptReadme = fs.readFileSync(".github/scripts/README.md", "utf8");
  assert.match(workflow, /^concurrency:\n  group: review-loop-\$\{\{ github\.event\.workflow_run\.pull_requests\[0\]\.number \}\}\n  cancel-in-progress: false\n  queue: max$/m);
  for (const documentation of [workflowCompanion, scriptReadme]) {
    assert.match(documentation, /up to 100 pending runs/i);
    assert.match(documentation, /queue (?:remains|is) bounded/i);
  }
  assert.match(workflow, /runAttempt: run\.run_attempt/);
  assert.match(workflow, /record\.runId === run\.id && record\.runAttempt === run\.run_attempt/);
  assert.match(workflow, /evaluator\.publishEvaluation/);
});

test("existing_failed_run_attempt_appends_nothing_reports_failure_and_sets_failed", async () => {
  const calls = { appended: 0, conclusions: [], failures: [], info: [] };
  const outcome = await publishEvaluation({
    result: { ok: false, message: "one blocker remains" },
    alreadyRecorded: true,
    appendJournal: async () => { calls.appended += 1; },
    reportCheck: async (conclusion) => { calls.conclusions.push(conclusion); },
    setFailed: (message) => { calls.failures.push(message); },
    info: (message) => { calls.info.push(message); },
  });

  assert.deepEqual(outcome, { appended: false, conclusion: "failure" });
  assert.equal(calls.appended, 0);
  assert.deepEqual(calls.conclusions, ["failure"]);
  assert.deepEqual(calls.failures, ["one blocker remains"]);
  assert.deepEqual(calls.info, []);
});

for (const refusal of [
  {
    state: "history-unverifiable",
    reason: "Configured App journal comment carries a malformed marker or JSON payload.",
    comments: [{ id: 999, body: `${JOURNAL_MARKER}{not-json} -->`, performed_via_github_app: { slug: TRUST.appSlug, id: TRUST.appId } }],
    options: {},
  },
  {
    state: "fork-refused",
    reason: "Fork pull requests deliberately fail closed: partial evidence is not evaluated.",
    comments: [],
    options: { headRepositoryId: 8 },
  },
]) {
  test(`trusted workflow reports ${refusal.state} without journaling`, async () => {
    const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
    const outcome = await executeTrustedWorkflow(repositoryRoot, "", refusal.comments, {
      expectJournal: false,
      ...refusal.options,
    });

    assert.equal(outcome.postedBody, undefined, "a refusal must not append an untrusted journal observation");
    assert.deepEqual(outcome.failures, [refusal.reason]);
    assert.equal(outcome.reportedCheck.name, "Review loop converged");
    assert.equal(outcome.reportedCheck.conclusion, "failure");
    assert.equal(outcome.reportedCheck.output.title, refusal.state);
    assert.equal(outcome.reportedCheck.output.summary, refusal.reason);
  });
}

test("trusted workflow publishes an exact ledger parse failure without journaling", async () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const parserMessage = "Review ledger: 'finding-1' is not a stable finding ID beginning with F- followed by at least three digits.";
  const body = ledger(["| finding-1 | 1 | advisory | advisory | naming |"], []);
  const outcome = await executeTrustedWorkflow(repositoryRoot, body, [], { expectJournal: false });

  assert.equal(outcome.postedBody, undefined);
  assert.deepEqual(outcome.failures, [parserMessage]);
  assert.equal(outcome.reportedCheck.name, "Review loop converged");
  assert.equal(outcome.reportedCheck.conclusion, "failure");
  assert.equal(outcome.reportedCheck.output.title, "evaluation-unavailable");
  assert.equal(outcome.reportedCheck.output.summary, parserMessage);
});

test("trusted workflow publishes evaluation-unavailable when the identified pull request cannot be fetched", async () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const outcome = await executeTrustedWorkflow(repositoryRoot, "", [], {
    expectJournal: false,
    pullGetError: new Error("transport failed"),
  });

  assert.equal(outcome.postedBody, undefined);
  assert.deepEqual(outcome.failures, ["Unable to read pull request 200: transport failed"]);
  assert.equal(outcome.reportedCheck.name, "Review loop converged");
  assert.equal(outcome.reportedCheck.head_sha, "ccc");
  assert.equal(outcome.reportedCheck.conclusion, "failure");
  assert.equal(outcome.reportedCheck.output.title, "evaluation-unavailable");
  assert.equal(outcome.reportedCheck.output.summary, "Unable to read pull request 200: transport failed");
});

for (const evidenceFailure of [
  { endpoint: "associatedPulls", label: "associated pull requests", summary: "Unable to read associated pull requests: transport failed" },
  { endpoint: "comments", label: "review comments", summary: "Unable to read review comments: transport failed" },
  { endpoint: "reviews", label: "pull request reviews", summary: "Unable to read pull request reviews: transport failed" },
  { endpoint: "jobs", label: "workflow jobs", summary: "Unable to read workflow jobs: transport failed" },
  { endpoint: "checks", label: "check runs", summary: "Unable to read check runs: transport failed" },
  { endpoint: "files", label: "changed files", summary: "Unable to read changed files: transport failed" },
]) {
  test(`trusted workflow publishes evaluation-unavailable when it cannot read ${evidenceFailure.label}`, async () => {
    const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
    const outcome = await executeTrustedWorkflow(repositoryRoot, "", [], {
      expectJournal: false,
      paginateError: { endpoint: evidenceFailure.endpoint, error: new Error("transport failed") },
    });

    assert.equal(outcome.postedBody, undefined);
    assert.deepEqual(outcome.failures, [evidenceFailure.summary]);
    assert.equal(outcome.reportedCheck.name, "Review loop converged");
    assert.equal(outcome.reportedCheck.head_sha, "ccc");
    assert.equal(outcome.reportedCheck.conclusion, "failure");
    assert.equal(outcome.reportedCheck.output.title, "evaluation-unavailable");
    assert.equal(outcome.reportedCheck.output.summary, evidenceFailure.summary);
  });
}

test("normal blocking evaluation still appends, reports failure and fails the workflow", async () => {
  const calls = { appended: 0, conclusions: [], failures: [] };
  const outcome = await publishEvaluation({
    result: { ok: false, state: "converging", message: "one blocker remains" },
    alreadyRecorded: false,
    appendJournal: async () => { calls.appended += 1; },
    reportCheck: async (conclusion) => { calls.conclusions.push(conclusion); },
    setFailed: (message) => { calls.failures.push(message); },
    info: () => {},
  });

  assert.deepEqual(outcome, { appended: true, conclusion: "failure" });
  assert.equal(calls.appended, 1);
  assert.deepEqual(calls.conclusions, ["failure"]);
  assert.deepEqual(calls.failures, ["one blocker remains"]);
});

test("converged evaluation still appends and reports success", async () => {
  const calls = { appended: 0, conclusions: [], failures: [] };
  const outcome = await publishEvaluation({
    result: { ok: true, state: "converged", message: "review loop converged" },
    alreadyRecorded: false,
    appendJournal: async () => { calls.appended += 1; },
    reportCheck: async (conclusion) => { calls.conclusions.push(conclusion); },
    setFailed: (message) => { calls.failures.push(message); },
    info: () => {},
  });

  assert.deepEqual(outcome, { appended: true, conclusion: "success" });
  assert.equal(calls.appended, 1);
  assert.deepEqual(calls.conclusions, ["success"]);
  assert.deepEqual(calls.failures, []);
});

test("journal-required evaluation rejects a missing journal writer before reporting", async () => {
  let reported = false;

  await assert.rejects(
    publishEvaluation({
      result: { ok: false, state: "converging", message: "one blocker remains" },
      alreadyRecorded: false,
      reportCheck: async () => { reported = true; },
      setFailed: () => {},
      info: () => {},
    }),
    /Evaluation state converging requires a journal writer/,
  );
  assert.equal(reported, false, "a journal-required result must not be reported as if publication completed");
});

test("only named required jobs with positive success converge; forks fail closed", () => {
  for (const conclusion of ["skipped", "neutral", undefined, "failure"]) {
    const jobs = [{ name: "Check, Test & Lint", status: "completed", conclusion }, { name: "Knowledge graph up to date", status: "completed", conclusion: "success" }];
    assert.equal(trusted({ jobs }).state, "converging");
  }
  assert.equal(trusted({ jobs: [{ name: "optional", status: "completed", conclusion: "failure" }, ...trusted().records.slice(0, 0), { name: "Check, Test & Lint", status: "completed", conclusion: "success" }, { name: "Knowledge graph up to date", status: "completed", conclusion: "success" }] }).state, "converged");
  assert.equal(trusted({ pull: { author: "author", headSha: "ccc", headRepositoryId: 8, baseRepositoryId: 7 } }).state, "fork-refused");
});

test("an empty journal without genesis authorization evaluates instead of refusing", () => {
  const result = trusted({ journalComments: [] });

  assert.equal(result.ok, false);
  assert.equal(result.state, "converging");
  assert.notEqual(result.state, "history-unverifiable");
  assert.equal(result.unauthenticatedAnchor, true);
});

test("an unauthenticated genesis is an open reified blocker and cannot converge", () => {
  const result = trusted({ journalComments: [] });

  assert.equal(result.blocking, 1);
  assert.deepEqual(result.syntheticFindings, [{
    id: "GENESIS",
    round: 1,
    reifiedBy: "verified genesis authorization",
    reified: true,
    state: "open",
    resolution: "",
    disposalError: "authorization reference is unreachable",
  }]);
  assert.equal(result.currentHash, loopStateHash({ diffSignature: "", openReifiedIds: ["GENESIS"], redChecks: [] }));
  assert.match(result.message, /GENESIS/);
});

test("synthetic GENESIS does not enter the ledger finding ID namespace", () => {
  const parsed = parseFindings("| ID | Round | Reified by | State | Resolution |\n|---|---|---|---|---|\n| GENESIS | 1 | anchor | open | |");
  const invalidRecord = makeJournalRecord({ ...TRUST, observation: 1, headSha: "ccc", stateHash: "one", blocking: 1, unauthenticatedAnchor: true, findingIds: ["GENESIS"], findings: [{ id: "GENESIS", reified: true, state: "open" }], ledgerFindings: [] });
  const invalidHistory = trusted({ journalComments: [journalComment(invalidRecord, TRUST)] });
  const empty = trusted({ journalComments: [] });

  assert.match(parsed.error, /not a stable finding ID/i);
  assert.equal(invalidHistory.state, "history-unverifiable");
  assert.match(invalidHistory.message, /not a finding ID/i);
  assert.deepEqual(empty.projectedFindings, []);
  assert.equal(empty.syntheticFindings[0].id, "GENESIS");
});

test("verified genesis authorization closes the synthetic blocker and permits convergence", () => {
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "GENESIS", state: "genesis", reason: "migrate this PR" })} -->`;
  const reference = { id: 93, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "COLLABORATOR", body };
  const evidence = { referenceId: 93, author: "maintainer", bodyDigest: sha256(body) };
  const result = trusted({ journalComments: [], genesisEvidence: evidence, references: [reference] });

  assert.equal(result.state, "converged");
  assert.equal(result.unauthenticatedAnchor, false);
  assert.deepEqual(result.syntheticFindings, []);
  assert.equal(result.blocking, 0);
});

test("a later verified genesis authorization authenticates an existing unauthenticated chain", () => {
  const firstEvaluation = trusted({ journalComments: [] });
  const firstRecord = makeJournalRecord({ ...TRUST, observation: 1, headSha: "bbb", stateHash: firstEvaluation.currentHash, blocking: firstEvaluation.blocking, unauthenticatedAnchor: firstEvaluation.unauthenticatedAnchor, findingIds: [], findings: [], ledgerFindings: [] });
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "GENESIS", state: "genesis", reason: "authenticate the existing chain" })} -->`;
  const reference = { id: 193, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "OWNER", body };
  const evidence = { referenceId: 193, author: "maintainer", bodyDigest: sha256(body) };
  const result = trusted({ journalComments: [journalComment(firstRecord, TRUST)], genesisEvidence: evidence, references: [reference] });

  assert.equal(firstEvaluation.unauthenticatedAnchor, true);
  assert.equal(result.state, "converged");
  assert.equal(result.unauthenticatedAnchor, false);
  assert.deepEqual(result.syntheticFindings, []);
});

function stagnantUnauthenticatedJournal() {
  const records = [];
  let priorDigest = null;
  for (let observation = 1; observation <= DEFAULT_STAGNATION_LIMIT; observation += 1) {
    const record = makeJournalRecord({
      ...TRUST,
      observation,
      headSha: `head-${observation}`,
      stateHash: `progress-free-${observation}`,
      blocking: 1,
      priorDigest,
      unauthenticatedAnchor: true,
      findingIds: [],
      findings: [],
      ledgerFindings: [],
    });
    records.push(journalComment(record, TRUST));
    priorDigest = record.digest;
  }
  return records;
}

test("a stalled unauthenticated chain accepts a stall record naming GENESIS", () => {
  const result = trusted({
    journalComments: stagnantUnauthenticatedJournal(),
    changedFiles: [STALLS_PATH],
    stallsContent: `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-08-04 | ${REPOSITORY}#${PULL_NUMBER} | GENESIS |\n`,
  });

  assert.equal(result.ok, false);
  assert.equal(result.state, "converging", result.message);
  assert.equal(result.unauthenticatedAnchor, true);
  assert.deepEqual(result.syntheticFindings.map((finding) => finding.id), ["GENESIS"]);
});

test("a stalled unauthenticated chain rejects a stall record omitting GENESIS", () => {
  const result = trusted({
    journalComments: stagnantUnauthenticatedJournal(),
    changedFiles: [STALLS_PATH],
    stallsContent: `## Open\n\n| Detected | Pull request | Stuck on |\n|---|---|---|\n| 2026-08-04 | ${REPOSITORY}#${PULL_NUMBER} | F-001 |\n`,
  });

  assert.equal(result.ok, false);
  assert.equal(result.state, "escalated");
  assert.match(result.message, /missing GENESIS/i);
});

test("genesis may evaluate unauthenticated while tombstones still require verified authorization", () => {
  assert.equal(trusted({ journalComments: [] }).state, "converging");
  const tombstoneBody = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "F-094", state: "tombstone", reason: "duplicate ID" })} -->`;
  const tombstoneReference = { id: 94, kind: "comment", repositoryId: 7, pullNumber: 200, user: { login: "maintainer" }, author_association: "MEMBER", body: tombstoneBody };
  const tombstone = { id: "F-094", evidence: { referenceId: 94, author: "maintainer", bodyDigest: sha256(tombstoneBody) } };
  assert.equal(trusted({ tombstones: [tombstone], references: [tombstoneReference] }).state, "converged");
  assert.equal(trusted({ tombstones: [tombstone], references: [] }).state, "history-unverifiable");
});

test("the embedded adapter journals an unauthenticated anchor and its real blocking state", async () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const outcome = await executeTrustedWorkflow(repositoryRoot, "", [], { expectJournal: false });

  assert.ok(outcome.postedBody, "an unauthenticated empty journal must still receive its genesis observation");
  const record = journalRecord({ body: outcome.postedBody });
  assert.equal(record.unauthenticatedAnchor, true);
  assert.deepEqual(record.findingIds, []);
  assert.deepEqual(record.findings, []);
  assert.deepEqual(record.ledgerFindings, []);
  assert.equal(record.blocking, 1);
  assert.equal(outcome.reportedCheck.output.title, "converging");
  assert.equal(outcome.reportedCheck.conclusion, "failure");
  assert.deepEqual(outcome.failures, [outcome.reportedCheck.output.summary]);
  assert.match(outcome.reportedCheck.output.summary, /GENESIS/);
});

test("the embedded adapter journals an authenticated genesis symmetrically", async () => {
  const repositoryRoot = process.env.REVIEW_LOOP_REPO || ".";
  const body = `${DIRECTIVE_MARKER}${JSON.stringify({ finding: "GENESIS", state: "genesis", reason: "anchor this PR" })} -->`;
  const evidence = { referenceId: 193, author: "maintainer", bodyDigest: sha256(body) };
  const pullBody = `<!-- keeplin-review-loop-metadata ${JSON.stringify({ genesisEvidence: evidence })} -->`;
  const reference = { id: 193, user: { login: "maintainer" }, author_association: "OWNER", body };
  const outcome = await executeTrustedWorkflow(repositoryRoot, pullBody, [reference], { expectJournal: false });

  assert.ok(outcome.postedBody, "an authenticated empty journal must receive its genesis observation");
  const record = journalRecord({ body: outcome.postedBody });
  assert.equal(record.unauthenticatedAnchor, false);
  assert.equal(record.blocking, 0);
  assert.deepEqual(outcome.failures, []);
  assert.equal(outcome.reportedCheck.output.title, "converged");
  assert.equal(outcome.reportedCheck.conclusion, "success");
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

test("trusted evaluator declares its exact least-privilege permission set", () => {
  const workflow = fs.readFileSync(".github/workflows/review-loop-evaluator.yml", "utf8");
  const lines = workflow.split("\n");
  const permissionHeaders = lines.filter((line) => line.trim() === "permissions:");
  const start = lines.indexOf("permissions:");
  const endOffset = lines.slice(start + 1).findIndex((line) => line.length === 0 || !line.startsWith("  "));

  assert.deepEqual(permissionHeaders, ["permissions:"], "the evaluator must declare one top-level permission block with no job override");
  assert.notEqual(start, -1, "the evaluator must declare top-level permissions");
  assert.notEqual(endOffset, -1, "the evaluator permission block must have a boundary");
  assert.deepEqual(lines.slice(start, start + 1 + endOffset), [
    "permissions:",
    "  actions: read",
    "  checks: write",
    "  contents: read",
    "  issues: write",
    "  pull-requests: write",
  ]);
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
  assert.match(ci, /^  checks: read$/m);
  assert.match(ci, /get_status=.*--write-out '%\{http_code\}'/);
  assert.match(ci, /test "\$\{get_status\}" = 200/);
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
