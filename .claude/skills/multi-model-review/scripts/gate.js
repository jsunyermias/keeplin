// Decide whether the adjudicator has cleared the pull request.
//
//   node gate.js <adjudicator-review.md> [--cycle N] [--max-cycles M]
//
// Exit 0  -> cleared; hand the PR to the final reviewer.
// Exit 1  -> not cleared; run another implement/review cycle.
// Exit 2  -> misuse, or the cycle cap was reached.
//
// Two signals are required together, not one. The phrase alone would let a
// reviewer advance an unreviewed change by emitting it in passing — including
// while quoting or arguing about it — and the verdict alone would drop the
// adjudicator's explicit control over when the cycle closes. Requiring both
// means an accident has to happen twice, in agreement, to slip through.
//
// Nothing here reads the review into the orchestrator's context: it reports a
// decision and a reason, never the findings.
const fs = require('fs');
const path = require('path');

const PHRASE = process.env.REVIEW_PHRASE || 'REVISION-COMPLETADA-SIN-BLOQUEANTES';

const argv = process.argv.slice(2);
const FILE = argv.find((a) => !a.startsWith('--'));

// Number() turns a missing or misspelt value into NaN, and every comparison
// against NaN is false — so `--cycle cinco` sailed past the cap check and the
// gate exited 1, "run another cycle", while reporting `"cycle": null`. An
// invalid invocation must be misuse (exit 2), never an invitation to iterate.
const bad = [];
const flag = (name, dflt) => {
  const i = argv.indexOf(`--${name}`);
  if (i === -1) return dflt;
  const raw = argv[i + 1];
  if (raw === undefined || raw.startsWith('--')) {
    bad.push(`--${name} needs a value`);
    return dflt;
  }
  if (!/^\d+$/.test(raw) || Number(raw) < 1) {
    bad.push(`--${name} must be an integer >= 1, got "${raw}"`);
    return dflt;
  }
  return Number(raw);
};
const CYCLE = flag('cycle', 1);
const MAX_CYCLES = flag('max-cycles', 5);
const rolesAt = argv.indexOf('--roles');
const ROLES = rolesAt === -1 ? null : argv[rolesAt + 1];
if (rolesAt !== -1 && (ROLES === undefined || ROLES.startsWith('--'))) {
  bad.push('--roles needs a value');
}
if (!bad.length && CYCLE > MAX_CYCLES) {
  bad.push(`--cycle ${CYCLE} is past --max-cycles ${MAX_CYCLES}`);
}

(() => {
  if (bad.length) {
    for (const b of bad) console.error(`bad argument: ${b}`);
    console.error('refusing to judge on an invalid invocation');
    process.exit(2);
  }
  if (!FILE) {
    console.error('usage: node gate.js <adjudicator-review.md> --roles <roles-prN.json> ' +
      '[--cycle N] [--max-cycles M]');
    process.exit(2);
  }
  if (!fs.existsSync(FILE)) {
    console.error(`no such review: ${FILE}`);
    process.exit(2);
  }

  // Whose review is this? A clean verdict from the implementer would otherwise
  // close a cycle, and the documentation admitted the gate did not check.
  if (ROLES) {
    if (!fs.existsSync(ROLES)) {
      console.error(`no such roles file: ${ROLES}`);
      process.exit(2);
    }
    const roles = JSON.parse(fs.readFileSync(ROLES, 'utf8'));
    const metaPath = `${FILE}.meta.json`;
    if (!fs.existsSync(metaPath)) {
      console.error(
        `${FILE} has no companion .meta.json, so the reviewer cannot be established. ` +
        'Rerun it through ask.js rather than accepting an unattributed review.'
      );
      process.exit(2);
    }
    const meta = JSON.parse(fs.readFileSync(metaPath, 'utf8'));
    if (meta.reviewer !== roles.adjudicator) {
      console.error(
        `refusing: this review is from "${meta.reviewer}" but pull request ${roles.pr} assigns ` +
        `adjudication to "${roles.adjudicator}"` +
        (meta.reviewer === roles.implementer ? ' — that is the implementer' : '')
      );
      process.exit(2);
    }
    // Matching the family is not enough. Two pull requests can share an
    // adjudicator, so a roles file from the wrong one passes the check above
    // and closes a cycle on a review of some other change entirely.
    if (meta.pr === undefined) {
      console.error(
        `${metaPath} records no pull request, so this review cannot be tied to ${roles.pr}. ` +
        'Rerun ask.js with --pr rather than accepting a review that could belong to another change.'
      );
      process.exit(2);
    }
    if (Number(meta.pr) !== Number(roles.pr)) {
      console.error(
        `refusing: this review is of pull request ${meta.pr} but the roles file is for ` +
        `${roles.pr}`
      );
      process.exit(2);
    }
  }

  const text = fs.readFileSync(FILE, 'utf8');

  // Take the verdict from the first line that declares one: a later mention is
  // usually the adjudicator discussing another reviewer's verdict, not its own.
  const verdict = (text.match(/^VEREDICTO:\s*(.+)$/m) || [])[1];
  const normalised = (verdict || '').trim().toUpperCase();
  const clean = normalised === 'SIN HALLAZGOS';

  // Substring matching is not good enough. The phrase appears in this
  // repository's own diff and in the prior reviews attached to the
  // adjudicator's prompt, and these UIs echo the prompt back into the reply —
  // so `includes` fires on a quotation. Require it as the exact final line,
  // and refuse if it appears more than once, which means it was discussed
  // rather than declared.
  // Count every appearance anywhere in the text, not just whole lines: a
  // mid-sentence mention alongside a well-formed final line would otherwise
  // clear, and these replies routinely echo a prompt that spells the phrase out.
  const total = text.split(PHRASE).length - 1;
  const lines = text.split('\n').map((l) => l.trim());
  const last = [...lines].reverse().find((l) => l !== '');
  const phrased = total === 1 && last === PHRASE;
  const quoted = total > 0 && !phrased;

  console.log(JSON.stringify({
    file: FILE,
    cycle: CYCLE,
    verdict: verdict ? verdict.trim() : null,
    phrase_present: phrased,
    phrase_quoted_not_declared: quoted,
    cleared: clean && phrased,
  }));

  if (clean && phrased) {
    console.error(`cleared on cycle ${CYCLE}`);
    process.exit(0);
  }

  if (!verdict) {
    console.error('no VEREDICTO line — the reviewer failed or the reply was cut short; ' +
      'this is not the same as a clean review');
  } else if (!clean) {
    console.error(`not cleared: verdict is "${verdict.trim()}"`);
  } else if (quoted) {
    console.error(`not cleared: "${PHRASE}" appears but not as the sole final line — ` +
      'that is a quotation, not a declaration');
  } else {
    console.error(`not cleared: verdict is clean but "${PHRASE}" is absent`);
  }

  if (CYCLE >= MAX_CYCLES) {
    console.error(`cycle cap reached (${CYCLE}/${MAX_CYCLES}) — stop and involve the maintainer ` +
      'rather than looping further');
    process.exit(2);
  }
  process.exit(1);
})();
