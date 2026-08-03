"use strict";

const { createHash } = require("node:crypto");
function checked(body, label) {
  return new RegExp(`^- \\[x\\] ${escapeRegex(label)}\\.?\\s*$`, "im").test(body || "");
}

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
const JOURNAL_SCHEMA = "keeplin.review-loop/v1";
const JOURNAL_MARKER = "<!-- keeplin-review-loop-journal ";
const DIRECTIVE_MARKER = "<!-- keeplin-review-loop-authorize ";
const AUTHORIZING_ASSOCIATIONS = new Set(["MEMBER", "OWNER", "COLLABORATOR"]);
const REQUIRED_JOBS = ["Check, Test & Lint", "Knowledge graph up to date"];

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function markedJson(body, marker) {
  const start = String(body || "").indexOf(marker);
  if (start < 0) return null;
  const end = String(body).indexOf(" -->", start);
  if (end < 0) return null;
  try { return JSON.parse(String(body).slice(start + marker.length, end)); } catch { return null; }
}

function verifyAuthorization({ finding, reference, pullAuthor, targetState, repositoryId, pullNumber }) {
  if (!reference) return { ok: false, reason: "authorization reference is unreachable" };
  if (reference.repositoryId !== repositoryId || reference.pullNumber !== pullNumber) return { ok: false, reason: "authorization reference is not bound to this repository and pull request" };
  const login = reference.user && reference.user.login;
  const association = String(reference.author_association || "").toUpperCase();
  if (!AUTHORIZING_ASSOCIATIONS.has(association)) return { ok: false, reason: "reference author is not authorized" };
  if (!login || login.toLowerCase() === String(pullAuthor || "").toLowerCase()) return { ok: false, reason: "pull-request author cannot authorize disposal" };
  if (reference.kind === "review" && String(reference.state || "").toUpperCase() === "DISMISSED") return { ok: false, reason: "authorization review was dismissed" };
  const directive = markedJson(reference.body, DIRECTIVE_MARKER);
  if (!directive || directive.finding !== finding.id || directive.state !== targetState || typeof directive.reason !== "string" || !directive.reason.trim()) {
    return { ok: false, reason: "reference has no matching machine-readable directive" };
  }
  const digest = sha256(reference.body || "");
  const evidence = finding.evidence || {};
  if (String(evidence.referenceId) !== String(reference.id) || evidence.author !== login || evidence.bodyDigest !== digest) {
    return { ok: false, reason: "recorded reference identity, author or body digest does not match" };
  }
  return { ok: true, reference: { id: reference.id, author: login, bodyDigest: digest, reason: directive.reason } };
}

function verifyResolvedCheck({ finding, checks, headSha, config }) {
  const evidence = finding.evidence || {};
  const check = (checks || []).find((candidate) => String(candidate.id) === String(evidence.checkRunId));
  if (!check) return { ok: false, reason: "resolution check is unreachable" };
  if (!REQUIRED_JOBS.includes(evidence.checkName) || check.name !== evidence.checkName || check.status !== "completed" || check.conclusion !== "success") return { ok: false, reason: "resolution check is not an explicitly required successful job" };
  if (check.head_sha !== headSha || check.workflow_id !== config.workflowId || check.workflow_run_id !== config.runId || check.app_slug !== config.appSlug || check.app_id !== config.appId) return { ok: false, reason: "resolution check is not bound to this head, workflow and App" };
  return { ok: true };
}

function verifyJournal(comments, config) {
  const records = [];
  for (const comment of comments || []) {
    const record = markedJson(comment.body, JOURNAL_MARKER);
    if (!record) continue;
    if (record.schema !== JOURNAL_SCHEMA || record.repositoryId !== config.repositoryId || record.workflowId !== config.workflowId || record.appSlug !== config.appSlug || record.appId !== config.appId || comment.app_slug !== config.appSlug || comment.app_id !== config.appId) return { ok: false, state: "history-unverifiable", message: "Journal producer, repository, workflow or schema does not match configuration." };
    const digest = sha256(canonicalJson({ ...record, digest: undefined }));
    if (record.digest !== digest) return { ok: false, state: "history-unverifiable", message: "Journal record was edited: its digest no longer matches." };
    records.push({ ...record, commentId: comment.id });
  }
  records.sort((a, b) => a.observation - b.observation);
  for (let index = 0; index < records.length; index += 1) {
    const prior = records[index - 1];
    if (records[index].observation !== index + 1 || records[index].priorDigest !== (prior ? prior.digest : null)) return { ok: false, state: "history-unverifiable", message: "Journal deletion or reordering is visible because a surviving descendant no longer matches its predecessor." };
  }
  return { ok: true, records };
}

function evaluateTrustedReviewLoop({ pull, findings = [], references = [], checks = [], journalComments = [], jobs = [], tombstones = [], genesisEvidence, changedFiles = [], stallsContent = "", diffSignature = "", stagnationLimit = DEFAULT_STAGNATION_LIMIT, config }) {
  if (pull.headRepositoryId !== pull.baseRepositoryId) return { ok: false, state: "fork-refused", message: "Fork pull requests deliberately fail closed: partial evidence is not evaluated." };
  const unreifiedOpen = findings.find((finding) => finding.state === "open" && !finding.reified);
  if (unreifiedOpen) return { ok: false, state: "history-unverifiable", message: `Review ledger: finding ${unreifiedOpen.id} is open but names no failing check.` };
  const reifiedAdvisory = findings.find((finding) => finding.state === ADVISORY && finding.reified);
  if (reifiedAdvisory) return { ok: false, state: "history-unverifiable", message: `Review ledger: finding ${reifiedAdvisory.id} is reified but has advisory state.` };
  const journal = verifyJournal(journalComments, config);
  if (!journal.ok) return journal;
  const priorRecord = journal.records[journal.records.length - 1];
  const currentIds = new Set(findings.map((finding) => finding.id));
  const tombstonedIds = new Set(tombstones.map((entry) => entry.id));
  const vanished = ((priorRecord && priorRecord.findingIds) || []).filter((id) => !currentIds.has(id) && !tombstonedIds.has(id));
  if (vanished.length > 0) return { ok: false, state: "history-unverifiable", message: `Finding IDs vanished without authorized tombstones: ${vanished.join(", ")}.` };
  const specialAuthorizations = [
    ...(journal.records.length === 0 ? [{ id: "GENESIS", state: "genesis", evidence: genesisEvidence }] : []),
    ...tombstones.map((entry) => ({ ...entry, state: "tombstone" })),
  ];
  for (const special of specialAuthorizations) {
    const reference = references.find((item) => String(item.id) === String(special.evidence && special.evidence.referenceId));
    const authorization = verifyAuthorization({ finding: special, reference, pullAuthor: pull.author, targetState: special.state, repositoryId: config.repositoryId, pullNumber: pull.number });
    if (!authorization.ok) return { ok: false, state: "history-unverifiable", message: `${special.id} lacks verified authorization: ${authorization.reason}.` };
  }
  const requiredFailures = REQUIRED_JOBS.filter((name) => {
    const matches = jobs.filter((job) => job.name === name);
    return matches.length !== 1 || matches[0].status !== "completed" || matches[0].conclusion !== "success";
  });
  // Reification is remembered across every surviving record, not just the newest one. Reading
  // only the newest record left a hole: an authorized tombstone retires an ID without reserving
  // it, and once a later record had been written without that finding, the ID could return as
  // advisory with no prior state to contradict it and converge unauthorized.
  const everReified = new Set();
  for (const record of journal.records) {
    if (!Array.isArray(record.findings)) continue;
    for (const item of record.findings) {
      if (item && item.reified) everReified.add(item.id);
    }
  }
  const projected = findings.map((finding) => {
    // This is still bounded by the truncation limit, and deliberately so: ADR 0008 does not
    // detect terminal truncation, so a shorter authentic prefix that never contained the
    // reifying record still allows an advisory classification to converge.
    const declassified = everReified.has(finding.id) && (!finding.reified || finding.state === ADVISORY);
    if (!["resolved", "dismissed"].includes(finding.state) && !declassified) return finding;
    const reference = references.find((item) => String(item.id) === String(finding.evidence && finding.evidence.referenceId));
    const authorization = verifyAuthorization({ finding, reference, pullAuthor: pull.author, targetState: finding.state, repositoryId: config.repositoryId, pullNumber: pull.number });
    if (!authorization.ok) return { ...finding, reified: true, state: "open", disposalError: authorization.reason };
    if (finding.state === "resolved") {
      const proof = verifyResolvedCheck({ finding, checks, headSha: pull.headSha, config });
      if (!proof.ok) return { ...finding, state: "open", disposalError: proof.reason };
    }
    return finding;
  });
  const open = projected.filter((finding) => finding.reified && finding.state === "open");
  const blockingNames = [...open.map((finding) => finding.id), ...requiredFailures].sort();
  const blocking = blockingNames.length;
  const currentHash = loopStateHash({ diffSignature, openReifiedIds: open.map((finding) => finding.id), redChecks: requiredFailures });
  const rounds = [...journal.records.map((record) => ({ round: record.observation, hash: record.stateHash, blocking: record.blocking })), { round: journal.records.length + 1, hash: currentHash, blocking }];
  const stalled = journal.records.length > 0 && blocking > 0 ? stagnationReason(rounds, currentHash, stagnationLimit) : "";
  if (stalled) {
    if (!changedFiles.includes(STALLS_PATH)) return { ok: false, state: "escalated", projectedFindings: projected, records: journal.records, currentHash, blocking, message: `Review loop stalled: ${stalled}. Record every blocker in ${STALLS_PATH}. History is verified only against tampering; an actor with repository write access can truncate it.` };
    const record = stallRecordsBlockers(stallsContent, config.repository, pull.number, blockingNames);
    if (!record.ok) return { ok: false, state: "escalated", projectedFindings: projected, records: journal.records, currentHash, blocking, message: `Review loop stalled: ${record.reason}. History is verified only against tampering; an actor with repository write access can truncate it.` };
  }
  const ok = blocking === 0;
  return {
    ok,
    state: ok ? "converged" : "converging",
    projectedFindings: projected,
    records: journal.records,
    currentHash,
    blocking,
    message: `${ok ? "Review loop converged" : "Review loop has not converged"}. History is verified only against tampering; an actor with repository write access can truncate it, and terminal truncation is not detected.`,
  };
}

function makeJournalRecord(fields) {
  const record = { schema: JOURNAL_SCHEMA, priorDigest: null, ...fields };
  record.digest = sha256(canonicalJson({ ...record, digest: undefined }));
  return record;
}

function journalComment(record, identity) {
  return { id: record.observation, app_slug: identity.appSlug, app_id: identity.appId, body: `${JOURNAL_MARKER}${JSON.stringify(record)} -->` };
}

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

function requireParsedFindings(parsed) {
  if (parsed && parsed.error) throw new Error(parsed.error);
  if (!parsed || !Array.isArray(parsed.findings)) throw new Error("Review ledger parser returned no findings result.");
  return parsed.findings;
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
  parseFindings,
  requireParsedFindings,
  redChecksFromRuns,
  requiredChecksFromNeeds,
  splitTableRow,
  stallMentionsPull,
  stallRecordsBlockers,
  AUTHORIZING_ASSOCIATIONS,
  DIRECTIVE_MARKER,
  JOURNAL_MARKER,
  JOURNAL_SCHEMA,
  REQUIRED_JOBS,
  canonicalJson,
  evaluateTrustedReviewLoop,
  journalComment,
  levelTwoSection,
  makeJournalRecord,
  sha256,
  verifyAuthorization,
  verifyJournal,
};
