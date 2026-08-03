"use strict";

const { createHash } = require("node:crypto");
const { checked } = require("./check-review-governance.js");

const LEDGER_HEADING = "Review ledger";
const ROUND_LOG_HEADING = "Round log";
const STALLS_PATH = "docs/review-stalls.md";
const DEFAULT_STAGNATION_LIMIT = 3;
const RESOLVED_CHECKBOX = "Blocking findings are resolved and conversations are closed";
const ADVISORY = "advisory";
const STATES = new Set(["open", "resolved", "dismissed", ADVISORY]);
const FAILING_CONCLUSIONS = new Set([
  "failure",
  "timed_out",
  "cancelled",
  "action_required",
  "stale",
]);

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function levelTwoSection(body, heading) {
  const start = new RegExp(`^##\\s+${escapeRegex(heading)}\\s*$`, "im").exec(body);
  if (!start) return null;

  const remainder = body.slice(start.index + start[0].length);
  const end = /^##[^#].*$/m.exec(remainder);
  return end ? remainder.slice(0, end.index) : remainder;
}

function levelThreeSection(text, heading) {
  const start = new RegExp(`^###\\s+${escapeRegex(heading)}\\s*$`, "im").exec(text);
  if (!start) return "";

  const remainder = text.slice(start.index + start[0].length);
  const end = /^###[^#].*$/m.exec(remainder);
  return end ? remainder.slice(0, end.index) : remainder;
}

function tableRows(text) {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("|") && line.endsWith("|"))
    .map((line) =>
      line
        .slice(1, -1)
        .split("|")
        .map((cell) => cell.trim()),
    )
    .filter((cells) => !cells.every((cell) => /^:?-{2,}:?$/.test(cell)))
    .filter((cells) => !/^id$/i.test(cells[0] || "") && !/^round$/i.test(cells[0] || ""));
}

function unquote(value) {
  return value.replace(/^`+|`+$/g, "").trim();
}

function isReified(reifiedBy) {
  const normalized = unquote(reifiedBy).toLowerCase();
  return normalized !== "" && normalized !== ADVISORY;
}

function parseFindings(section) {
  const findings = [];
  const seen = new Set();

  for (const cells of tableRows(section.split(/^###/m)[0])) {
    const [id = "", round = "", reifiedBy = "", state = "", resolution = ""] = cells;
    if (!id) continue;

    if (!/^F-\d{3,}$/.test(id)) {
      return { error: `Review ledger: '${id}' is not a stable finding ID of the form F-001.` };
    }
    if (seen.has(id)) {
      return { error: `Review ledger: finding ID ${id} appears more than once.` };
    }
    seen.add(id);

    const roundNumber = Number.parseInt(round, 10);
    if (!Number.isInteger(roundNumber) || roundNumber < 1) {
      return { error: `Review ledger: finding ${id} has no valid round number.` };
    }

    const normalizedState = state.toLowerCase();
    if (!STATES.has(normalizedState)) {
      return {
        error:
          `Review ledger: finding ${id} has state '${state}'. ` +
          `Use one of: open, resolved, dismissed, advisory.`,
      };
    }

    const reified = isReified(reifiedBy);

    if (!reified && normalizedState === "open") {
      return {
        error:
          `Review ledger: finding ${id} is open but names no failing check. ` +
          `Reify it as a test, property, contract assertion or check-docs check, ` +
          `or record it as advisory.`,
      };
    }
    if (normalizedState === "dismissed" && !resolution) {
      return {
        error:
          `Review ledger: finding ${id} is dismissed without a cited reason. ` +
          `Cite the priority decision or the accepted ADR that settles it.`,
      };
    }

    findings.push({
      id,
      round: roundNumber,
      reifiedBy: unquote(reifiedBy),
      state: normalizedState,
      resolution,
      reified,
    });
  }

  return { findings };
}

function parseRoundLog(section) {
  const rounds = [];

  for (const cells of tableRows(levelThreeSection(section, ROUND_LOG_HEADING))) {
    const [round = "", hash = "", blocking = ""] = cells;
    if (!round) continue;

    const roundNumber = Number.parseInt(round, 10);
    const blockingCount = Number.parseInt(blocking, 10);
    const normalizedHash = unquote(hash).toLowerCase();

    if (!Number.isInteger(roundNumber) || roundNumber < 1) {
      return { error: `Round log: '${round}' is not a valid round number.` };
    }
    if (!/^[0-9a-f]{64}$/.test(normalizedHash)) {
      return { error: `Round log: round ${roundNumber} has no valid 64-hex loop-state hash.` };
    }
    if (!Number.isInteger(blockingCount) || blockingCount < 0) {
      return { error: `Round log: round ${roundNumber} has no valid blocking-set size.` };
    }

    rounds.push({ round: roundNumber, hash: normalizedHash, blocking: blockingCount });
  }

  for (let i = 1; i < rounds.length; i += 1) {
    if (rounds[i].round <= rounds[i - 1].round) {
      return { error: `Round log: rounds must ascend; round ${rounds[i].round} does not.` };
    }
  }

  return { rounds };
}

function diffSignatureFromFiles(files) {
  return (files || [])
    .map((file) => `${file.filename}\0${file.status || ""}\0${file.sha || ""}`)
    .sort()
    .join("\n");
}

function redChecksFromRuns(runs, ignoreNames = []) {
  const ignored = new Set(ignoreNames);
  return [
    ...new Set(
      (runs || [])
        .filter(
          (run) =>
            run.status === "completed" &&
            FAILING_CONCLUSIONS.has(run.conclusion) &&
            !ignored.has(run.name),
        )
        .map((run) => run.name),
    ),
  ].sort();
}

function loopStateHash({ diffSignature = "", openReifiedIds = [], redChecks = [] }) {
  return createHash("sha256")
    .update(diffSignature)
    .update("\x1e")
    .update([...openReifiedIds].sort().join(","))
    .update("\x1e")
    .update([...redChecks].sort().join(","))
    .digest("hex");
}

function stallMentionsPull(stallsContent, repository, pullNumber) {
  const escapedRepository = escapeRegex(repository);
  const number = String(pullNumber);
  return (
    new RegExp(`github\\.com\\/${escapedRepository}\\/pull\\/${number}(?:\\D|$)`, "i").test(
      stallsContent,
    ) || new RegExp(`${escapedRepository}#${number}(?:\\D|$)`, "i").test(stallsContent)
  );
}

function describeBlockers(openReified, redChecks) {
  const parts = openReified.map((finding) => `${finding.id} (${finding.reifiedBy})`);
  parts.push(...redChecks.map((name) => `red check '${name}'`));
  return parts.join("; ");
}

function stagnationReason(rounds, currentHash, stagnationLimit) {
  const priorHashes = rounds.slice(0, -1).map((entry) => entry.hash);
  if (priorHashes.includes(currentHash)) {
    const repeated = rounds.find((entry) => entry.hash === currentHash);
    return `the loop state is byte-identical to round ${repeated.round}`;
  }

  let streak = 0;
  for (let i = 1; i < rounds.length; i += 1) {
    streak = rounds[i].blocking < rounds[i - 1].blocking ? 0 : streak + 1;
  }
  if (streak >= stagnationLimit) {
    return `the blocking set has not shrunk for ${streak} rounds (limit ${stagnationLimit})`;
  }

  return "";
}

function evaluateReviewLoop({
  body,
  changedFiles = [],
  stallsContent = "",
  repository,
  pullNumber,
  redChecks = [],
  diffSignature = "",
  stagnationLimit = DEFAULT_STAGNATION_LIMIT,
}) {
  const section = levelTwoSection(body || "", LEDGER_HEADING);
  if (section === null) {
    return {
      ok: false,
      state: "malformed",
      message: `The pull request body has no '${LEDGER_HEADING}' section from the repository template.`,
    };
  }

  const parsedFindings = parseFindings(section);
  if (parsedFindings.error) {
    return { ok: false, state: "malformed", message: parsedFindings.error };
  }
  const parsedRounds = parseRoundLog(section);
  if (parsedRounds.error) {
    return { ok: false, state: "malformed", message: parsedRounds.error };
  }

  const { findings } = parsedFindings;
  const { rounds } = parsedRounds;
  const sortedRedChecks = [...new Set(redChecks)].sort();
  const openReified = findings
    .filter((finding) => finding.reified && finding.state === "open")
    .sort((a, b) => a.id.localeCompare(b.id));
  const blocking = openReified.length + sortedRedChecks.length;
  const currentHash = loopStateHash({
    diffSignature,
    openReifiedIds: openReified.map((finding) => finding.id),
    redChecks: sortedRedChecks,
  });

  if (findings.length > 0 && rounds.length === 0) {
    return {
      ok: false,
      state: "malformed",
      message:
        `Review ledger records ${findings.length} finding(s) but the round log is empty. ` +
        `Record the round with loop-state hash ${currentHash} and blocking set ${blocking}.`,
    };
  }

  if (rounds.length > 0) {
    const last = rounds[rounds.length - 1];
    if (last.hash !== currentHash) {
      return {
        ok: false,
        state: "malformed",
        message:
          `Round log round ${last.round} records loop-state hash ${last.hash}, but the ` +
          `current state hashes to ${currentHash}. Append the current round.`,
      };
    }
    if (last.blocking !== blocking) {
      return {
        ok: false,
        state: "malformed",
        message:
          `Round log round ${last.round} records a blocking set of ${last.blocking}, but ` +
          `${blocking} item(s) block: ${describeBlockers(openReified, sortedRedChecks) || "none"}.`,
      };
    }
  }

  if (blocking > 0 && checked(body || "", RESOLVED_CHECKBOX)) {
    return {
      ok: false,
      state: "malformed",
      message:
        `'${RESOLVED_CHECKBOX}' is ticked while ${blocking} item(s) still block: ` +
        `${describeBlockers(openReified, sortedRedChecks)}. Convergence is required checks ` +
        `green and zero open reified findings, never a reviewer's satisfaction.`,
    };
  }

  const stalled = rounds.length > 0 ? stagnationReason(rounds, currentHash, stagnationLimit) : "";
  if (stalled && blocking > 0) {
    const blockers = describeBlockers(openReified, sortedRedChecks);
    if (!changedFiles.includes(STALLS_PATH)) {
      return {
        ok: false,
        state: "escalated",
        message:
          `Review loop stalled: ${stalled}. Escalate to the maintainer naming what is stuck ` +
          `(${blockers}) and record it in ${STALLS_PATH} in this pull request. ` +
          `Iterating further without that record is prohibited.`,
      };
    }
    if (!stallMentionsPull(stallsContent, repository, pullNumber)) {
      return {
        ok: false,
        state: "escalated",
        message:
          `Review loop stalled: ${stalled}. ${STALLS_PATH} must identify this exact pull ` +
          `request (${repository}#${pullNumber}) and what is stuck (${blockers}).`,
      };
    }
    return {
      ok: false,
      state: "escalated",
      message:
        `Review loop stalled and escalated to the maintainer: ${stalled}. Stuck on ${blockers}. ` +
        `Recorded in ${STALLS_PATH}; the maintainer resolves it by dismissing the finding with ` +
        `a cited reason or by fixing what fails.`,
    };
  }

  if (blocking === 0) {
    return {
      ok: true,
      state: "converged",
      message:
        `Review loop converged: required checks are green and no reified finding is open ` +
        `(${findings.length} finding(s) recorded, ${
          findings.filter((finding) => finding.state === ADVISORY).length
        } advisory).`,
    };
  }

  return {
    ok: false,
    state: "converging",
    message:
      `Review loop has not converged: ${blocking} item(s) block — ` +
      `${describeBlockers(openReified, sortedRedChecks)}. The blocking set must shrink strictly ` +
      `each round.`,
  };
}

module.exports = {
  DEFAULT_STAGNATION_LIMIT,
  LEDGER_HEADING,
  STALLS_PATH,
  diffSignatureFromFiles,
  evaluateReviewLoop,
  loopStateHash,
  redChecksFromRuns,
  stallMentionsPull,
};
