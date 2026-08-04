"use strict";

const fs = require("node:fs");
const evaluator = require("./check-review-loop.js");

function option(name) {
  const index = process.argv.indexOf(`--${name}`);
  if (index < 0 || index + 1 >= process.argv.length) throw new Error(`Missing --${name}.`);
  return process.argv[index + 1];
}

function safeInteger(name) {
  const value = Number(option(name));
  if (!Number.isSafeInteger(value)) throw new Error(`--${name} must be a safe integer.`);
  return value;
}

function readJson(path) {
  return JSON.parse(fs.readFileSync(path, "utf8"));
}

function main() {
  const pull = readJson(option("pull"));
  const comments = readJson(option("comments")).map((comment) => ({
    ...comment,
    app_slug: comment.app_slug || (comment.performed_via_github_app && comment.performed_via_github_app.slug),
    app_id: comment.app_id || (comment.performed_via_github_app && comment.performed_via_github_app.id),
  }));
  const parsed = evaluator.parseFindings(evaluator.levelTwoSection(pull.body || "", evaluator.LEDGER_HEADING) || "");
  const replayFindings = evaluator.requireParsedFindings(parsed).map((finding) => {
    try {
      return { ...finding, evidence: JSON.parse(finding.resolution) };
    } catch {
      return finding;
    }
  });
  const result = evaluator.verifyTerminalMalformedRecovery({
    comments,
    candidateCommentId: option("candidate-comment-id"),
    replayFindings,
    config: {
      repositoryId: safeInteger("repository-id"),
      workflowId: safeInteger("workflow-id"),
      appSlug: option("app-slug"),
      appId: safeInteger("app-id"),
    },
  });
  if (!result.ok) throw new Error(result.message);
  process.stdout.write(`${JSON.stringify({
    ok: true,
    candidateCommentId: result.record.commentId,
    observation: result.record.observation,
    priorDigest: result.record.priorDigest,
    digest: result.record.digest,
    findings: result.record.findings || [],
  }, null, 2)}\n`);
}

try {
  main();
} catch (error) {
  process.stderr.write(`Recovery refused: ${error.message}\n`);
  process.exitCode = 1;
}
