"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { evaluateReviewGovernance } = require("./check-review-governance.js");

const reviewedBody = `
## Independent review

- Reviewer (human or model family): Codex
- Implementer (human or model family): Claude
- [x] Reviewer is independent from the implementer.
- [x] Blocking findings are resolved and conversations are closed.
- Review evidence/link:
  https://github.com/jsunyermias/keeplin/pull/200#pullrequestreview-123

- Maintainer waiver (leave empty unless the maintainer set the review aside here):
  - Where the maintainer said so:
  - What goes unreviewed:
  - Entry in \`docs/review-debt.md\`:
  - Follow-up issue or sweep that will carry the deferred review:

## Merge readiness
`;

function evaluate(body, overrides = {}) {
  return evaluateReviewGovernance({
    body,
    changedFiles: [],
    debtContent: "",
    repository: "jsunyermias/keeplin",
    pullNumber: 200,
    ...overrides,
  });
}

test("accepts an independent review with evidence", () => {
  const result = evaluate(reviewedBody);
  assert.equal(result.ok, true);
  assert.equal(result.path, "review");
});

test("rejects a review without evidence", () => {
  const result = evaluate(reviewedBody.replace(/https:\/\/github\.com\/[^\s]+/, ""));
  assert.equal(result.ok, false);
});

test("rejects a self-review", () => {
  const result = evaluate(reviewedBody.replace("Codex", "Claude"));
  assert.equal(result.ok, false);
});

test("rejects a missing review and an incomplete waiver", () => {
  const result = evaluate(reviewedBody.replace("Codex", "").replace(/https:\/\/github\.com\/[^\s]+/, ""));
  assert.equal(result.ok, false);
  assert.match(result.message, /complete every maintainer-waiver field/i);
});

const waivedBody = `
## Independent review

- Reviewer (human or model family):
- Implementer (human or model family): Claude
- [ ] Reviewer is independent from the implementer.
- [ ] Blocking findings are resolved and conversations are closed.
- Review evidence/link:

- Maintainer waiver (leave empty unless the maintainer set the review aside here):
  - Where the maintainer said so: https://github.com/jsunyermias/keeplin/pull/200#issuecomment-123
  - What goes unreviewed: Persistence failure paths and rollback behavior.
  - Entry in \`docs/review-debt.md\`: Added for jsunyermias/keeplin#200.
  - Follow-up issue or sweep that will carry the deferred review: https://github.com/jsunyermias/keeplin/issues/201

## Merge readiness
`;

test("requires the debt file to change for a waiver", () => {
  const result = evaluate(waivedBody);
  assert.equal(result.ok, false);
  assert.match(result.message, /requires docs\/review-debt\.md to change/i);
});

test("requires the debt entry to name the exact pull request", () => {
  const result = evaluate(waivedBody, {
    changedFiles: ["docs/review-debt.md"],
    debtContent: "Open debt for https://github.com/jsunyermias/keeplin/pull/199",
  });
  assert.equal(result.ok, false);
  assert.match(result.message, /exact pull request/i);
});

test("accepts a complete waiver with matching review debt", () => {
  const result = evaluate(waivedBody, {
    changedFiles: ["docs/review-debt.md"],
    debtContent: "Open debt for https://github.com/jsunyermias/keeplin/pull/200.",
  });
  assert.equal(result.ok, true);
  assert.equal(result.path, "waiver");
});

test("rejects a body without the review section", () => {
  const result = evaluate("## Summary\n\nNothing else.");
  assert.equal(result.ok, false);
  assert.match(result.message, /no 'Independent review' section/i);
});
