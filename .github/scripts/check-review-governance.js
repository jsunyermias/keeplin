"use strict";

const REVIEW_HEADING = "Independent review";
const READINESS_HEADING = "Merge readiness";
const DEBT_PATH = "docs/review-debt.md";

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function section(body, startHeading, endHeading) {
  const start = new RegExp(`^##\\s+${escapeRegex(startHeading)}\\s*$`, "im").exec(body);
  if (!start) return "";

  const remainder = body.slice(start.index + start[0].length);
  const end = new RegExp(`^##\\s+${escapeRegex(endHeading)}\\s*$`, "im").exec(remainder);
  return end ? remainder.slice(0, end.index) : remainder;
}

function fieldValue(text, label) {
  const match = new RegExp(
    `^\\s*-\\s*${escapeRegex(label)}\\s*:\\s*(.*?)\\s*$`,
    "im",
  ).exec(text);
  return match ? match[1].trim() : "";
}

function meaningful(value) {
  if (!value) return false;
  const normalized = value.trim().toLowerCase();
  return !(
    /^(?:n\/?a|none|no|pending|pendiente|tbd|todo|unknown|desconocido|-|—)$/.test(normalized) ||
    /^<[^>]+>$/.test(normalized)
  );
}

function checked(text, label) {
  return new RegExp(
    `^\\s*-\\s*\\[[xX]\\]\\s*${escapeRegex(label)}(?:\\s|\\.|$)`,
    "im",
  ).test(text);
}

function reviewEvidence(reviewSection) {
  const reviewedPart = reviewSection.split(/^\s*-\s*Maintainer waiver\b/im)[0];
  const evidenceStart = reviewedPart.search(/^\s*-\s*Review evidence\/link\s*:/im);
  if (evidenceStart < 0) return "";
  const evidence = reviewedPart.slice(evidenceStart);
  const match = /https:\/\/github\.com\/[^\s)>]+/i.exec(evidence);
  return match ? match[0] : "";
}

function debtMentionsPull(debtContent, repository, pullNumber) {
  const escapedRepository = escapeRegex(repository);
  const number = String(pullNumber);
  return (
    new RegExp(`github\\.com\\/${escapedRepository}\\/pull\\/${number}(?:\\D|$)`, "i").test(debtContent) ||
    new RegExp(`${escapedRepository}#${number}(?:\\D|$)`, "i").test(debtContent)
  );
}

function evaluateReviewGovernance({ body, changedFiles, debtContent, repository, pullNumber }) {
  const reviewSection = section(body || "", REVIEW_HEADING, READINESS_HEADING);
  if (!reviewSection) {
    return {
      ok: false,
      message: "The pull request body has no 'Independent review' section from the repository template.",
    };
  }

  const reviewer = fieldValue(reviewSection, "Reviewer (human or model family)");
  const implementer = fieldValue(reviewSection, "Implementer (human or model family)");
  const independent = checked(reviewSection, "Reviewer is independent from the implementer");
  const findingsResolved = checked(
    reviewSection,
    "Blocking findings are resolved and conversations are closed",
  );
  const evidence = reviewEvidence(reviewSection);

  if (
    meaningful(reviewer) &&
    meaningful(implementer) &&
    reviewer.toLowerCase() !== implementer.toLowerCase() &&
    independent &&
    findingsResolved &&
    evidence
  ) {
    return { ok: true, path: "review", message: `Independent review recorded at ${evidence}.` };
  }

  const waiverFields = [
    "Where the maintainer said so",
    "What goes unreviewed",
    "Entry in `docs/review-debt.md`",
    "Follow-up issue or sweep that will carry the deferred review",
  ];
  const missingWaiverFields = waiverFields.filter(
    (label) => !meaningful(fieldValue(reviewSection, label)),
  );

  if (missingWaiverFields.length > 0) {
    return {
      ok: false,
      message:
        "Record a complete independent review, or complete every maintainer-waiver field. " +
        `Missing: ${missingWaiverFields.join(", ")}.`,
    };
  }

  if (!changedFiles.includes(DEBT_PATH)) {
    return {
      ok: false,
      message: `A maintainer waiver requires ${DEBT_PATH} to change in the same pull request.`,
    };
  }

  if (!debtMentionsPull(debtContent || "", repository, pullNumber)) {
    return {
      ok: false,
      message: `${DEBT_PATH} must identify this exact pull request (${repository}#${pullNumber}).`,
    };
  }

  return {
    ok: true,
    path: "waiver",
    message: `Maintainer waiver and review-debt entry recorded for ${repository}#${pullNumber}.`,
  };
}

module.exports = {
  DEBT_PATH,
  debtMentionsPull,
  evaluateReviewGovernance,
  meaningful,
  section,
};
