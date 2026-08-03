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

function splitTableRow(line) {
  const cells = [];
  let cell = "";
  const content = line.slice(1, hasUnescapedTerminalPipe(line) ? -1 : undefined);
  for (let index = 0; index < content.length; index += 1) {
    const character = content[index];
    if (character !== "\\") {
      if (character === "|") {
        cells.push(cell.trim());
        cell = "";
      } else cell += character;
      continue;
    }
    let end = index;
    while (content[end] === "\\") end += 1;
    const count = end - index;
    cell += "\\".repeat(Math.floor(count / 2));
    if (content[end] === "|") {
      if (count % 2 === 1) cell += "|";
      else {
        cells.push(cell.trim());
        cell = "";
      }
      index = end;
    } else {
      if (count % 2 === 1) cell += "\\";
      index = end - 1;
    }
  }
  cells.push(cell.trim());
  return cells;
}

function hasUnescapedTerminalPipe(line) {
  let backslashes = 0;
  for (let index = line.length - 2; index >= 0 && line[index] === "\\"; index -= 1) {
    backslashes += 1;
  }
  return line.endsWith("|") && backslashes % 2 === 0;
}

function tableRows(text) {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("|"))
    .map(splitTableRow)
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
  return JSON.stringify(
    (files || [])
      .map((file) => [file.filename, file.status || "", file.sha || ""])
      .sort((left, right) => {
        const leftJson = JSON.stringify(left);
        const rightJson = JSON.stringify(right);
        return leftJson < rightJson ? -1 : leftJson > rightJson ? 1 : 0;
      }),
  );
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

// A check that has not completed is not a green check. Reporting only completed failures
// turned "unknown" into "passing": with nothing yet finished, the failure set was empty and
// the loop declared the required checks green before any of them had run. Convergence needs
// positive evidence, so anything not completed-and-successful is reported and blocks.
function pendingChecksFromRuns(runs, ignoreNames = []) {
  const ignored = new Set(ignoreNames);
  return [
    ...new Set(
      (runs || [])
        .filter((run) => run.status !== "completed" && !ignored.has(run.name))
        .map((run) => run.name),
    ),
  ].sort();
}

function requiredChecksFromNeeds(needs) {
  const names = { test: "Check, Test & Lint", graph: "Knowledge graph up to date" };
  return Object.entries(names).reduce(
    (result, [job, name]) => {
      const conclusion = needs && needs[job] && needs[job].result;
      if (conclusion !== "success") result.push(name);
      return result;
    },
    [],
  );
}

// Both separators must be bytes the inputs cannot contain, or distinct loop states
// collide. A comma is not such a byte: check-run names routinely contain one — this
// repository's own required check is literally "Check, Test & Lint" — so joining a list
// on commas made {"a,b", "c"} and {"a", "b,c"} hash identically.
function loopStateHash({ diffSignature = "", openReifiedIds = [], redChecks = [] }) {
  const canonical = JSON.stringify({
    diffSignature,
    openReifiedIds: [...openReifiedIds].sort(),
    redChecks: [...redChecks].sort(),
  });
  return createHash("sha256")
    .update(canonical)
    .digest("hex");
}

function stallMentionsPull(text, repository, pullNumber) {
  const escapedRepository = escapeRegex(repository);
  const number = String(pullNumber);
  return (
    new RegExp(`github\\.com\\/${escapedRepository}\\/pull\\/${number}(?:\\D|$)`, "i").test(text) ||
    new RegExp(`${escapedRepository}#${number}(?:\\D|$)`, "i").test(text)
  );
}

// A stall record has to say what is stuck, not merely mention the pull request somewhere
// in the file. Matching the whole document accepted an old `Cleared` row, or a passing
// reference in the prose, as if the maintainer had been told what to look at.
function stallRecordsBlockers(stallsContent, repository, pullNumber, blockerNames) {
  const open = levelTwoSection(stallsContent || "", "Open");
  if (open === null) {
    return { ok: false, reason: `${STALLS_PATH} has no '## Open' section` };
  }

  for (const cells of tableRows(open)) {
    const row = cells.join(" ");
    if (!stallMentionsPull(row, repository, pullNumber)) continue;

    const stuckOn = cells[2] || "";
    const tokens = stuckOn
      .split(/<br\s*\/?>/i)
      .map((token) => unquote(token).trim())
      .filter(Boolean);
    const missing = blockerNames.filter((name) => !tokens.includes(name));
    if (missing.length > 0) {
      return {
        ok: false,
        reason:
          `its '## Open' entry does not name what is stuck (missing ${missing.join(", ")})`,
      };
    }
    return { ok: true };
  }

  return {
    ok: false,
    reason: `no row under '## Open' names ${repository}#${pullNumber}`,
  };
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
  pendingChecks = [],
  diffSignature = "",
  stagnationLimit = DEFAULT_STAGNATION_LIMIT,
}) {
  // ADR 0004's migration contract: a pull request opened before the ledger existed has no
  // such section, and is treated as round zero rather than retroactively invalidated. It
  // still has to pass its required checks; it simply carries no findings yet.
  const section = levelTwoSection(body || "", LEDGER_HEADING) ?? "";

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
    // Bytewise, not localeCompare: that comparator varies with the runtime's ICU version and
    // locale, and ADR 0006 requires the two repositories to produce byte-identical results.
    // The hash already sorts bytewise, so this only reached message and blocker ordering —
    // but a determinism requirement that holds only where it is load-bearing is not one.
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
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
    const blockerNames = [
      ...openReified.map((finding) => finding.id),
      ...sortedRedChecks,
    ];
    const record = stallRecordsBlockers(stallsContent, repository, pullNumber, blockerNames);
    if (!record.ok) {
      return {
        ok: false,
        state: "escalated",
        message:
          `Review loop stalled: ${stalled}. ${STALLS_PATH} must identify this exact pull ` +
          `request (${repository}#${pullNumber}) and what is stuck (${blockers}) — ` +
          `${record.reason}.`,
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

  // Not-yet-finished checks are reported separately from red ones: they keep the pull
  // request from converging without entering the hashed blocking set, which would otherwise
  // churn the round log on every re-run.
  const sortedPendingChecks = [...new Set(pendingChecks)].sort();
  if (blocking === 0 && sortedPendingChecks.length > 0) {
    return {
      ok: false,
      state: "awaiting-checks",
      message:
        `Review loop has not converged: no reified finding is open, but ` +
        `${sortedPendingChecks.length} required check(s) have not finished — ` +
        `${sortedPendingChecks.map((name) => `'${name}'`).join(", ")}. ` +
        `A check that has not completed is not a green check.`,
    };
  }

  if (blocking === 0) {
    return {
      ok: true,
      state: "converged",
      message:
        `Review loop converged: its explicit workflow dependencies succeeded and no reified ` +
        `finding is open ` +
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
  pendingChecksFromRuns,
  redChecksFromRuns,
  requiredChecksFromNeeds,
  splitTableRow,
  stallMentionsPull,
  stallRecordsBlockers,
};
