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

function normalizedCommentAttribution(comment) {
  const nested = comment && comment.performed_via_github_app;
  if (!nested) return comment;
  const topSlug = comment.app_slug;
  const topId = comment.app_id;
  if ((topSlug !== undefined && topSlug !== null && topSlug !== nested.slug)
      || (topId !== undefined && topId !== null && String(topId) !== String(nested.id))) {
    throw new Error(`Comment ${comment.id} carries conflicting top-level and nested App attribution.`);
  }
  return { ...comment, app_slug: nested.slug, app_id: nested.id };
}

function chronologicallyOrderedComments(comments) {
  let priorTimestamp = null;
  let priorId = null;
  return comments.map((comment) => {
    if (!comment || !Number.isSafeInteger(comment.id)) throw new Error("Every recovery comment must carry a safe integer ID.");
    const timestamp = Date.parse(comment.created_at || "");
    if (!Number.isFinite(timestamp)) throw new Error(`Comment ${comment.id} carries no valid created_at timestamp.`);
    if (priorTimestamp !== null && (timestamp < priorTimestamp || comment.id <= priorId)) {
      throw new Error("Recovery comments are not in chronological created_at and ID order.");
    }
    priorTimestamp = timestamp;
    priorId = comment.id;
    return normalizedCommentAttribution(comment);
  });
}

function main() {
  const pull = readJson(option("pull"));
  const inputComments = readJson(option("comments"));
  if (!Array.isArray(inputComments)) throw new Error("Recovery comments input must be an array.");
  const comments = chronologicallyOrderedComments(inputComments);
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
    findings: result.restorableFindings,
    projectedFindings: result.record.findings || [],
  }, null, 2)}\n`);
}

try {
  main();
} catch (error) {
  process.stderr.write(`Recovery refused: ${error.message}\n`);
  process.exitCode = 1;
}
