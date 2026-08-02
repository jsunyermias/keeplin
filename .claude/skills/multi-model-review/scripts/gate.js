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

const PHRASE = process.env.REVIEW_PHRASE || 'REVISION-COMPLETADA-SIN-BLOQUEANTES';

const argv = process.argv.slice(2);
const FILE = argv.find((a) => !a.startsWith('--'));
const flag = (name, dflt) => {
  const i = argv.indexOf(`--${name}`);
  return i === -1 ? dflt : Number(argv[i + 1]);
};
const CYCLE = flag('cycle', 1);
const MAX_CYCLES = flag('max-cycles', 5);

(() => {
  if (!FILE) {
    console.error('usage: node gate.js <adjudicator-review.md> [--cycle N] [--max-cycles M]');
    process.exit(2);
  }
  if (!fs.existsSync(FILE)) {
    console.error(`no such review: ${FILE}`);
    process.exit(2);
  }

  const text = fs.readFileSync(FILE, 'utf8');

  // Take the verdict from the first line that declares one: a later mention is
  // usually the adjudicator discussing another reviewer's verdict, not its own.
  const verdict = (text.match(/^VEREDICTO:\s*(.+)$/m) || [])[1];
  const normalised = (verdict || '').trim().toUpperCase();
  const clean = normalised === 'SIN HALLAZGOS';
  const phrased = text.includes(PHRASE);

  console.log(JSON.stringify({
    file: FILE,
    cycle: CYCLE,
    verdict: verdict ? verdict.trim() : null,
    phrase_present: phrased,
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
