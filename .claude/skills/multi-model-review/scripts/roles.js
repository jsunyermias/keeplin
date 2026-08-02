// Decide who implements and who reviews a pull request.
//
//   node roles.js <pr-number> [implementer]      -> JSON on stdout
//
// Qwen and GLM always review: they never implement, so they are never asked to
// judge their own work. The third seat is taken by whichever of Kimi and Codex
// did not implement.
//
// The implementer is fixed for the whole life of a pull request, not rotated
// per cycle. Rotating it within a review was a real defect: `collect.js` hands
// over the diff accumulated since the merge base, so a model that implemented
// cycle 1 would later adjudicate a diff containing its own work — exactly what
// "whoever implements never reviews" forbids. Alternation happens across pull
// requests instead, which is where it actually buys independence.
//
// The assignment is derived from the pull request number rather than randomised
// so a reader can reconstruct, months later, which family judged which change.
const ROTATING = ['kimi', 'codex'];
const FIXED_REVIEWERS = ['qwen', 'glm'];

const PR = Number(process.argv[2]);
const OVERRIDE = process.argv[3];

(() => {
  if (!Number.isInteger(PR) || PR < 1) {
    throw new Error(`pull request number must be an integer >= 1, got "${process.argv[2]}"`);
  }
  if (OVERRIDE && !ROTATING.includes(OVERRIDE)) {
    throw new Error(`implementer must be one of ${ROTATING.join(', ')}, got "${OVERRIDE}"`);
  }

  // Even pull requests go to Kimi, odd ones to Codex; an explicit override wins
  // when a human already assigned the work.
  const implementer = OVERRIDE || ROTATING[PR % 2];
  const adjudicator = ROTATING[(ROTATING.indexOf(implementer) + 1) % 2];

  console.log(JSON.stringify({
    pr: PR,
    implementer,
    // Run these two first, in parallel, neither seeing the other.
    independent: FIXED_REVIEWERS,
    // Then this one, with both prior reviews attached.
    adjudicator,
    reviewers: [...FIXED_REVIEWERS, adjudicator],
    note: 'implementer stays fixed for every cycle of this pull request',
  }, null, 2));
})();
