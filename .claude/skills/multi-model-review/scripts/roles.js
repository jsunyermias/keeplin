// Decide who implements and who reviews a pull request, and hold that decision.
//
//   node roles.js <pr-number> [implementer] [--dir <path>]
//
// Qwen and GLM always review: they never implement, so they are never asked to
// judge their own work. The third seat is taken by whichever of Kimi and Codex
// did not implement.
//
// The assignment is written to roles-pr<N>.json on first use and refused if a
// later call contradicts it. Determinism alone was not enough: the override
// argument let a second cycle name a different implementer, which promoted the
// previous one to adjudicator over a cumulative diff containing its own work —
// the exact thing "whoever implements never reviews" forbids.
//
// It is derived from the pull request number rather than randomised so a reader
// can reconstruct, months later, which family judged which change.
const fs = require('fs');
const path = require('path');

const ROTATING = ['kimi', 'codex'];
const FIXED_REVIEWERS = ['qwen', 'glm'];

const argv = process.argv.slice(2);
const dirAt = argv.indexOf('--dir');
const DIR = dirAt === -1 ? process.cwd() : argv[dirAt + 1];
const positional = argv.filter((a, i) => !a.startsWith('--') && i !== dirAt + 1);
const PR = Number(positional[0]);
const OVERRIDE = positional[1];

(() => {
  if (!Number.isInteger(PR) || PR < 1) {
    throw new Error(`pull request number must be an integer >= 1, got "${positional[0]}"`);
  }
  if (OVERRIDE && !ROTATING.includes(OVERRIDE)) {
    throw new Error(`implementer must be one of ${ROTATING.join(', ')}, got "${OVERRIDE}"`);
  }

  const file = path.join(DIR, `roles-pr${PR}.json`);
  const stored = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : null;

  // Even pull requests go to Kimi, odd ones to Codex; an override only applies
  // the first time, when a human is recording who actually took the work.
  const implementer = stored ? stored.implementer : (OVERRIDE || ROTATING[PR % 2]);

  if (stored && OVERRIDE && OVERRIDE !== stored.implementer) {
    throw new Error(
      `pull request ${PR} is already assigned to ${stored.implementer}; refusing to reassign to ` +
      `${OVERRIDE}. Changing it mid-review would let ${stored.implementer} adjudicate a diff ` +
      `containing its own work. Delete ${file} only if the whole review is starting over.`
    );
  }

  const adjudicator = ROTATING[(ROTATING.indexOf(implementer) + 1) % 2];
  const assignment = {
    pr: PR,
    implementer,
    // Run these two first, in parallel, neither seeing the other.
    independent: FIXED_REVIEWERS,
    // Then this one, with both prior reviews attached.
    adjudicator,
    reviewers: [...FIXED_REVIEWERS, adjudicator],
    note: 'assignment is recorded on first use and holds for every cycle of this pull request',
  };

  if (!stored) fs.writeFileSync(file, JSON.stringify(assignment, null, 2) + '\n');
  console.log(JSON.stringify(assignment, null, 2));
})();
