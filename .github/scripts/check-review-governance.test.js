"use strict";
const test = require("node:test");
const assert = require("node:assert");
const g = require("./check-review-governance.js");

const MARKERS = ["// md:impl FsBackend > fn apply_format_migration"];
const REPO = "jsunyermias/keeplin";

function body(o = {}) {
  const d = { rev: "Codex", imp: "Claude", tick: true,
    ev: "https://github.com/jsunyermias/keeplin/pull/1#issuecomment-9",
    marker: "`// md:impl FsBackend > fn apply_format_migration`",
    inv: "FORMAT_VERSION stays 8", revert: "Sí", exc: null, waiver: null, ...o };
  return [
    "## Summary", "",
    `- Companion marker preserved: ${d.marker}`,
    `- Invariant it preserves: ${d.inv}`,
    `- Test fails when the fix is reverted: ${d.revert}`,
    d.exc ? "- Soft-rail exception invoked: Sí" : "- Soft-rail exception invoked: No",
    d.exc ? `- Step comment ids: ${d.exc}` : "",
    "", "## Independent review", "",
    `- Reviewer (human or model family): ${d.rev}`,
    `- Implementer (human or model family): ${d.imp}`,
    `- [${d.tick ? "x" : " "}] Reviewer is independent from the implementer.`,
    "- [x] Blocking findings are resolved and conversations are closed.",
    `- Review evidence/link: ${d.ev}`, "",
    "- Maintainer waiver (leave empty unless the maintainer set the review aside here):",
    `  - Where the maintainer said so:${d.waiver ? " " + d.waiver : ""}`,
    `  - What goes unreviewed:${d.waiver ? " el parser" : ""}`,
    `  - Entry in \`docs/review-debt.md\`:${d.waiver ? " fila" : ""}`,
    `  - Follow-up issue or sweep that will carry the deferred review:${d.waiver ? " #187" : ""}`,
    "", "## Merge readiness",
  ].join("\n");
}
const run = (b, extra = {}) => g.evaluateReviewGovernance({
  body: b, changedFiles: [], debtContent: "", repository: REPO, pullNumber: 1,
  comments: [], repoMarkers: MARKERS, ...extra });
const cm = (...pairs) => pairs.map(([id, text]) => ({ id, body: text }));

test("accepts a cross-family review with every field answered", () => {
  assert.equal(run(body()).ok, true);
});

// Vulnerability 3 — model families
test("rejects identical names", () => assert.equal(run(body({ rev: "Claude", imp: "Claude" })).ok, false));
test("rejects same family under different names", () => {
  for (const [rev, imp] of [["Claude Opus 5", "Claude"], ["Opus", "Sonnet"], ["GPT-5", "Codex"], ["Anthropic", "Haiku"]]) {
    assert.equal(run(body({ rev, imp })).ok, false, `${rev} vs ${imp}`);
  }
});
test("accepts genuinely different families", () => {
  for (const [rev, imp] of [["Kimi", "Claude"], ["Codex", "Kimi"], ["human maintainer", "Claude"]]) {
    assert.equal(run(body({ rev, imp })).ok, true, `${rev} vs ${imp}`);
  }
});
test("rejects an unrecognised family name", () => {
  assert.match(run(body({ rev: "yo mismo", imp: "Claude" })).message, /Unrecognised/);
});

// The line-bleed regression: a blank field must never inherit the next line.
test("a blank Reviewer does not inherit the Implementer line", () => {
  assert.equal(g.fieldValue("- Reviewer (human or model family):\n- Implementer (human or model family): Codex",
    "Reviewer (human or model family)"), "");
  assert.equal(run(body({ rev: "" })).ok, false);
});
test("a blank waiver field does not inherit the next one", () => {
  const b = body({ rev: "", waiver: null });
  assert.match(run(b).message, /Missing:/);
});

// Vulnerability 2 — companion citation
test("requires a real marker and an invariant", () => {
  assert.equal(run(body({ marker: "" })).ok, false);
  assert.equal(run(body({ inv: "" })).ok, false);
  assert.match(run(body({ marker: "`// md:fn does_not_exist`" })).message, /does not exist/);
  assert.match(run(body({ marker: "`storage/fs.rs`" })).message, /is not a `\/\/ md:` marker/);
});
test("skips the citation when the pull request touches no code", () => {
  assert.equal(run(body({ marker: "", inv: "", revert: "" }), { touchesCode: false }).ok, true);
});

// Vulnerability 5 — revert proof
test("requires a valid revert answer", () => {
  assert.equal(run(body({ revert: "" })).ok, false);
  assert.equal(run(body({ revert: "creo que sí" })).ok, false);
  for (const a of ["Sí", "Si", "No", "No aplica", "N/A", "Sí — el test rojo está en el hilo"]) {
    assert.equal(run(body({ revert: a })).ok, true, a);
  }
});

// Vulnerability 1 — sequential exception protocol
test("accepts three distinct maintainer comments in order", () => {
  assert.equal(run(body({ exc: "11, 22, 33" }), { comments: cm([11, g.EXCEPTION_STEPS[0]], [22, g.EXCEPTION_STEPS[1]], [33, g.EXCEPTION_STEPS[2]]) }).ok, true);
});
test("refuses a batch: all three phrases in one comment", () => {
  const r = run(body({ exc: "11" }), { comments: cm([11, g.EXCEPTION_STEPS.join("\n")]) });
  assert.equal(r.ok, false);
  assert.match(r.message, /Batching is refused/);
});
test("refuses steps posted out of order", () => {
  const r = run(body({ exc: "11, 22, 33" }), { comments: cm([22, g.EXCEPTION_STEPS[0]], [11, g.EXCEPTION_STEPS[1]], [33, g.EXCEPTION_STEPS[2]]) });
  assert.match(r.message, /out of order/);
});
test("refuses a missing step", () => {
  const r = run(body({ exc: "11, 22" }), { comments: cm([11, g.EXCEPTION_STEPS[0]], [22, g.EXCEPTION_STEPS[1]]) });
  assert.match(r.message, /no maintainer comment contains/);
});
test("requires the body to link every step comment", () => {
  const r = run(body({ exc: "11, 33" }), { comments: cm([11, g.EXCEPTION_STEPS[0]], [22, g.EXCEPTION_STEPS[1]], [33, g.EXCEPTION_STEPS[2]]) });
  assert.match(r.message, /must link the comment/);
});
test("ignores the protocol when no exception is invoked", () => {
  assert.equal(run(body()).ok, true);
});

// Waiver path, unchanged behaviour
test("waiver path requires the registry to change", () => {
  const b = body({ rev: "", waiver: "sesión del 2026-08-01" });
  assert.match(run(b, { changedFiles: [] }).message, /to change in the same pull request/);
});

test("waiver path requires the registry to name this pull request", () => {
  const b = body({ rev: "", waiver: "sesión del 2026-08-01" });
  assert.match(run(b, { changedFiles: ["docs/review-debt.md"], debtContent: "" }).message,
    /must identify this exact pull request/);
});

test("waiver path accepts a complete waiver with its registry entry", () => {
  const b = body({ rev: "", waiver: "sesión del 2026-08-01" });
  assert.equal(run(b, { changedFiles: ["docs/review-debt.md"],
    debtContent: "https://github.com/jsunyermias/keeplin/pull/1" }).ok, true);
});
