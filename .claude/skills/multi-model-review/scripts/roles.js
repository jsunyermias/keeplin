// Decide who implements and who reviews a pull request, and hold that decision.
//
//   node roles.js <pr-number> --dir <review-package> [implementer]
//
// Qwen and GLM always review: they never implement, so they are never asked to
// judge their own work. The third seat is taken by whichever of Kimi and Codex
// did not implement.
//
// The implementer may also be a name outside the rotation — `claude`, or a
// person. That is not an edge case, it is what happens whenever the change did
// not come from the ping-pong at all, and the earlier version could not say it:
// it forced every pull request onto Kimi or Codex, so a review of someone
// else's work either recorded a false implementer or ran with the independence
// check switched off. Both happened. When the implementer is outside the
// rotation neither Kimi nor Codex is conflicted, and the third seat is picked
// by parity like any other.
//
// The assignment is written to roles-pr<N>.json on first use and refused if a
// later call contradicts it. Determinism alone was not enough: the override
// argument let a second cycle name a different implementer, which promoted the
// previous one to adjudicator over a cumulative diff containing its own work —
// the exact thing "whoever implements never reviews" forbids.
//
// --dir is required rather than defaulting to the current directory. A default
// meant the file landed wherever the operator happened to be standing, so the
// same pull request run from two directories produced two independent
// assignments and the persistence above guarded nothing.
const fs = require('fs');
const path = require('path');

const ROTATING = ['kimi', 'codex'];
const FIXED_REVIEWERS = ['qwen', 'glm'];

const { parse } = require('./args');

const ARGS = parse(process.argv.slice(2), {
  name: 'roles.js',
  positionals: ['pr-number'],
  optionalPositionals: ['implementer'],
  options: { dir: {} },
  usage: 'roles.js <pr-number> --dir <review-package> [implementer]',
});
// The implementer is optional, so it is accepted as a trailing positional the
// spec does not require; parse() would reject an extra one, hence the explicit
// second slot below.
const PR = Number(ARGS.positional[0]);
const DIR = ARGS.dir;
// Names are compared, so they are normalised first. Without this, `Codex` was
// not equal to `codex`: it was recorded as an implementer from outside the
// rotation, and parity then handed adjudication to codex — the same family
// judging its own work, which is the one thing the assignment exists to stop.
const OVERRIDE = ARGS.positional[1] ? ARGS.positional[1].trim().toLowerCase() : undefined;

(() => {
  if (!Number.isInteger(PR) || PR < 1) {
    throw new Error(
      `pull request number must be an integer >= 1, got "${ARGS.positional[0]}"`);
  }
  if (!DIR) {
    throw new Error(
      '--dir is required: it is what makes the assignment stick to this review rather ' +
      'than to whichever directory the command was run from.\n' +
      'usage: node roles.js <pr-number> --dir <review-package> [implementer]'
    );
  }
  if (!fs.existsSync(DIR)) {
    throw new Error(`no such directory: ${DIR}. Run collect.js first.`);
  }
  if (OVERRIDE && FIXED_REVIEWERS.includes(OVERRIDE)) {
    throw new Error(
      `${OVERRIDE} is a fixed reviewer and never implements; naming it here would leave the ` +
      'pull request with a reviewer judging its own work.'
    );
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

  // Outside the rotation, both rotating families are free to judge, so the seat
  // is decided by parity exactly as it would have been anyway.
  const external = !ROTATING.includes(implementer);
  const adjudicator = external
    ? ROTATING[PR % 2]
    : ROTATING[(ROTATING.indexOf(implementer) + 1) % 2];

  const assignment = {
    pr: PR,
    implementer,
    implementer_in_rotation: !external,
    // Run these two first, in parallel, neither seeing the other.
    independent: FIXED_REVIEWERS,
    // Then this one, with both prior reviews attached.
    adjudicator,
    reviewers: [...FIXED_REVIEWERS, adjudicator],
    // Scoped to the directory, and says so. Persistence is per --dir, not per
    // pull request: a new package gets a new assignment, and a note that
    // promised otherwise would mislead exactly the operator who relied on it.
    note: 'recorded on first use in this directory, and holds for every cycle run against it; '
      + 'a different --dir is a separate assignment',
  };

  if (!stored) fs.writeFileSync(file, JSON.stringify(assignment, null, 2) + '\n');
  console.log(JSON.stringify(assignment, null, 2));
})();
