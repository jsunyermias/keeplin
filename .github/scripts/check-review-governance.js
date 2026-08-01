"use strict";

const REVIEW_HEADING = "Independent review";
const READINESS_HEADING = "Merge readiness";
const DEBT_PATH = "docs/review-debt.md";

// Model families. Two agents of the same family never review each other, however
// they name themselves. Every alias of a family maps to the same canonical token.
const MODEL_FAMILIES = {
  claude: ["claude", "anthropic", "opus", "sonnet", "haiku"],
  codex: ["codex", "openai", "gpt", "o1", "o3"],
  kimi: ["kimi", "moonshot"],
  gemini: ["gemini", "bard", "google"],
  llama: ["llama", "meta"],
  mistral: ["mistral", "mixtral"],
  human: ["human", "maintainer", "mantenedor", "persona"],
};

const EXCEPTION_STEPS = [
  "EXCEPCIÓN ACEPTADA PASO 1",
  "EXCEPCIÓN ACEPTADA PASO 2",
  "EXCEPCIÓN ACEPTADA PASO 3",
];

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

// A field value never spans lines. `\s` matches newlines, so the previous
// `\s*:\s*(.*?)\s*$` silently captured the NEXT line when a field was left
// blank — a blank `Reviewer` was read as the `Implementer` line and passed.
function fieldValue(text, label) {
  const match = new RegExp(
    `^[^\\S\\r\\n]*-[^\\S\\r\\n]*${escapeRegex(label)}[^\\S\\r\\n]*:[^\\S\\r\\n]*([^\\r\\n]*?)[^\\S\\r\\n]*$`,
    "im",
  ).exec(text);
  return match ? match[1].trim() : "";
}

function meaningful(value) {
  if (!value) return false;
  const normalized = value.trim().toLowerCase();
  return !(
    /^(?:n\/?a|none|no|ninguno|ninguna|nadie|pending|pendiente|tbd|todo|unknown|desconocido|-|—|–|\?+|\.\.\.)$/
      .test(normalized) || /^<[^>]+>$/.test(normalized)
  );
}

function checked(text, label) {
  return new RegExp(
    `^[^\\S\\r\\n]*-[^\\S\\r\\n]*\\[[xX]\\][^\\S\\r\\n]*${escapeRegex(label)}(?:\\s|\\.|$)`,
    "im",
  ).test(text);
}

// Canonical family for a declared name, or null when it matches no known family.
// Unknown names fail closed: "yo mismo" or "el autor" must never satisfy the rule.
function modelFamily(value) {
  const normalized = String(value || "").toLowerCase();
  const hits = new Set();
  for (const [family, aliases] of Object.entries(MODEL_FAMILIES)) {
    if (aliases.some((alias) => normalized.includes(alias))) hits.add(family);
  }
  return hits.size === 1 ? [...hits][0] : null;
}

function reviewEvidence(reviewSection) {
  const reviewedPart = reviewSection.split(/^\s*-\s*Maintainer waiver\b/im)[0];
  const start = reviewedPart.search(/^[^\S\r\n]*-[^\S\r\n]*Review evidence\/link[^\S\r\n]*:/im);
  if (start < 0) return "";
  const match = /https:\/\/github\.com\/[^\s)>]+/i.exec(reviewedPart.slice(start));
  return match ? match[0] : "";
}

function debtMentionsPull(debtContent, repository, pullNumber) {
  const repo = escapeRegex(repository);
  const n = String(pullNumber);
  return (
    new RegExp(`github\\.com\\/${repo}\\/pull\\/${n}(?:\\D|$)`, "i").test(debtContent) ||
    new RegExp(`${repo}#${n}(?:\\D|$)`, "i").test(debtContent)
  );
}

// The three exception steps must arrive as three DISTINCT maintainer comments in
// ascending id order. A single comment containing all three phrases is a batch and
// is rejected: the protocol asks for three separate decisions, not one paste.
function evaluateException({ body, comments }) {
  const declared = /^[^\S\r\n]*-[^\S\r\n]*Soft-rail exception invoked[^\S\r\n]*:[^\S\r\n]*(s[ií]|yes)[^\S\r\n]*$/im.test(body);
  if (!declared) return { ok: true, invoked: false };

  const found = [];
  for (const phrase of EXCEPTION_STEPS) {
    const hit = (comments || []).find((c) => String(c.body || "").includes(phrase));
    if (!hit) {
      return { ok: false, invoked: true, message: `Exception invoked but no maintainer comment contains "${phrase}".` };
    }
    found.push({ phrase, id: Number(hit.id) });
  }
  const ids = found.map((f) => f.id);
  if (new Set(ids).size !== ids.length) {
    return {
      ok: false, invoked: true,
      message: "The three exception steps share a comment. Batching is refused: each step needs its own maintainer comment.",
    };
  }
  for (let i = 1; i < ids.length; i++) {
    if (ids[i] <= ids[i - 1]) {
      return {
        ok: false, invoked: true,
        message: `Exception steps are out of order: "${found[i].phrase}" was posted before "${found[i - 1].phrase}".`,
      };
    }
  }
  for (const { phrase, id } of found) {
    if (!body.includes(String(id))) {
      return {
        ok: false, invoked: true,
        message: `The pull request body must link the comment carrying "${phrase}" (id ${id}) for traceability.`,
      };
    }
  }
  return { ok: true, invoked: true, ids };
}

// The implementer must quote a real `// md:` marker and the invariant it preserves.
// A fabricated marker fails: the marker must exist in the repository.
function evaluateCompanionCitation({ body, repoMarkers }) {
  const marker = fieldValue(body, "Companion marker preserved");
  const invariant = fieldValue(body, "Invariant it preserves");
  if (!meaningful(marker)) {
    return { ok: false, message: "Quote the `// md:` marker of the companion your change preserves." };
  }
  if (!meaningful(invariant)) {
    return { ok: false, message: "State the companion invariant your change preserves." };
  }
  const cited = marker.replace(/`/g, "").trim();
  if (!/^\/\/\s*md:/.test(cited)) {
    return { ok: false, message: `"${cited}" is not a \`// md:\` marker.` };
  }
  if (repoMarkers && repoMarkers.length && !repoMarkers.includes(cited)) {
    return { ok: false, message: `Marker "${cited}" does not exist in the repository.` };
  }
  return { ok: true };
}

// A green test proves nothing unless it fails without the fix.
function evaluateRevertProof({ body }) {
  // Not routed through meaningful(): "No" is a legitimate answer here and a
  // filler value everywhere else. This field is validated against its own set.
  const answer = fieldValue(body, "Test fails when the fix is reverted").trim();
  if (answer === "") {
    return { ok: false, message: "Answer 'Test fails when the fix is reverted' with Sí / No / No aplica." };
  }
  if (!/^(s[ií]|yes|no aplica|not applicable|n\/?a|no)(\s*[—–:,.-].*)?$/i.test(answer.trim())) {
    return { ok: false, message: `"${answer}" is not one of Sí / No / No aplica.` };
  }
  return { ok: true };
}

function evaluateReviewGovernance({
  body, changedFiles, debtContent, repository, pullNumber,
  comments = [], repoMarkers = [], touchesCode = true,
}) {
  const reviewSection = section(body || "", REVIEW_HEADING, READINESS_HEADING);
  if (!reviewSection) {
    return { ok: false, message: "The pull request body has no 'Independent review' section from the repository template." };
  }

  const exception = evaluateException({ body: body || "", comments });
  if (!exception.ok) return { ok: false, message: exception.message };

  if (touchesCode) {
    const citation = evaluateCompanionCitation({ body: body || "", repoMarkers });
    if (!citation.ok) return { ok: false, message: citation.message };
    const revert = evaluateRevertProof({ body: body || "" });
    if (!revert.ok) return { ok: false, message: revert.message };
  }

  const reviewer = fieldValue(reviewSection, "Reviewer (human or model family)");
  const implementer = fieldValue(reviewSection, "Implementer (human or model family)");
  const independent = checked(reviewSection, "Reviewer is independent from the implementer");
  const findingsResolved = checked(reviewSection, "Blocking findings are resolved and conversations are closed");
  const evidence = reviewEvidence(reviewSection);

  if (meaningful(reviewer) && meaningful(implementer) && independent && findingsResolved && evidence) {
    const reviewerFamily = modelFamily(reviewer);
    const implementerFamily = modelFamily(implementer);
    if (!reviewerFamily || !implementerFamily) {
      return {
        ok: false,
        message: `Declare a known model family for both roles. Unrecognised: ${
          !reviewerFamily ? `reviewer "${reviewer}"` : `implementer "${implementer}"`
        }. Known families: ${Object.keys(MODEL_FAMILIES).join(", ")}.`,
      };
    }
    if (reviewerFamily === implementerFamily) {
      return {
        ok: false,
        message: `Reviewer "${reviewer}" and implementer "${implementer}" are both the ${reviewerFamily} family. A family does not review itself.`,
      };
    }
    return { ok: true, path: "review", message: `Independent review recorded at ${evidence} (${implementerFamily} implemented, ${reviewerFamily} reviewed).` };
  }

  const waiverFields = [
    "Where the maintainer said so",
    "What goes unreviewed",
    "Entry in `docs/review-debt.md`",
    "Follow-up issue or sweep that will carry the deferred review",
  ];
  const missing = waiverFields.filter((label) => !meaningful(fieldValue(reviewSection, label)));
  if (missing.length > 0) {
    return {
      ok: false,
      message: `Record a complete independent review, or complete every maintainer-waiver field. Missing: ${missing.join(", ")}.`,
    };
  }
  if (!changedFiles.includes(DEBT_PATH)) {
    return { ok: false, message: `A maintainer waiver requires ${DEBT_PATH} to change in the same pull request.` };
  }
  if (!debtMentionsPull(debtContent || "", repository, pullNumber)) {
    return { ok: false, message: `${DEBT_PATH} must identify this exact pull request (${repository}#${pullNumber}).` };
  }
  return { ok: true, path: "waiver", message: `Maintainer waiver and review-debt entry recorded for ${repository}#${pullNumber}.` };
}

module.exports = {
  DEBT_PATH, EXCEPTION_STEPS, MODEL_FAMILIES,
  debtMentionsPull, evaluateCompanionCitation, evaluateException,
  evaluateReviewGovernance, evaluateRevertProof, fieldValue, meaningful,
  modelFamily, section,
};
