// Decide who implements and who reviews for a given cycle.
//
//   node roles.js <cycle-number>            -> JSON on stdout
//
// Qwen and GLM always review: they never implement, so they are never asked to
// judge their own work. The third seat alternates between Kimi and Codex, and
// whichever of the two implements a cycle does not review it — that is the
// whole point of the rotation, and it is what AGENTS.md means by the
// implementer not being the only reviewer.
//
// The assignment is derived from the cycle number rather than randomised so a
// reader can reconstruct, months later, which family judged which change.
const CYCLE = Number(process.argv[2]);

const FIXED_REVIEWERS = ['qwen', 'glm'];
const ROTATING = ['kimi', 'codex'];

(() => {
  if (!Number.isInteger(CYCLE) || CYCLE < 1) {
    throw new Error(`cycle must be an integer >= 1, got "${process.argv[2]}"`);
  }

  // Cycle 1 -> kimi implements, codex reviews. Cycle 2 -> the reverse.
  const implementer = ROTATING[(CYCLE - 1) % 2];
  const counterpart = ROTATING[CYCLE % 2];

  console.log(JSON.stringify({
    cycle: CYCLE,
    implementer,
    reviewers: [...FIXED_REVIEWERS, counterpart],
  }, null, 2));
})();
