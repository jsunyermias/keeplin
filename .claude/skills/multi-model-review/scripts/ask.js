// Send a prompt file to one model and write its reply to a file.
//
//   node ask.js <qwen|glm|kimi|codex> <prompt-file> <out-file>
//
// This is the messenger primitive: prompt and reply both travel as files, so a
// whole review cycle runs without that text passing through the orchestrating
// agent's context. Only the verdict line is ever read back.
//
// Run from a workspace holding node_modules and the session files each browser
// driver needs (kimi-state.json, zai-state.json, qwen-state.json). Codex needs
// no session file — it uses ~/.codex/auth.json written by `codex login`.
const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

const [REVIEWER, PROMPT, OUT] = process.argv.slice(2);

// A driver that hangs would hang the pipeline with it: spawnSync blocks and
// there is nothing to report. Bound it, and say plainly that a timeout is not
// a review.
const TIMEOUT_MS = Number(process.env.ASK_TIMEOUT_MS || 20 * 60 * 1000);

const SKILLS = path.dirname(path.dirname(__dirname));
const MODELS = {
  qwen: process.env.QWEN_MODEL || 'Qwen3.8-Max-Preview',
  glm: process.env.GLM_MODEL || 'GLM-5.2',
  kimi: process.env.KIMI_MODEL || 'K3',
  codex: process.env.CODEX_MODEL || 'gpt-5.6-sol',
};

const DRIVERS = {
  qwen: ['qwen-web-chat', 'qwen.js'],
  glm: ['glm-web-chat', 'glm.js'],
  kimi: ['kimi-web-chat', 'kimi.js'],
};

(() => {
  if (!REVIEWER || !PROMPT || !OUT) {
    throw new Error('usage: node ask.js <qwen|glm|kimi|codex> <prompt-file> <out-file>');
  }
  if (!fs.existsSync(PROMPT)) throw new Error(`no such prompt file: ${PROMPT}`);
  if (!MODELS[REVIEWER]) throw new Error(`unknown reviewer: ${REVIEWER}`);

  const env = {
    ...process.env,
    // The drivers live outside the workspace; point Node and the shell at the
    // workspace's own installs rather than requiring a copy beside each skill.
    NODE_PATH: [path.resolve('node_modules'), process.env.NODE_PATH]
      .filter(Boolean).join(path.delimiter),
    PATH: [path.resolve('node_modules/.bin'), process.env.PATH]
      .filter(Boolean).join(path.delimiter),
  };

  let run;
  if (REVIEWER === 'codex') {
    // read-only keeps a reviewing agent from editing the tree it is judging;
    // skip-git-repo-check lets it run from a scratch workspace.
    run = spawnSync('codex', [
      'exec', '-c', `model="${MODELS.codex}"`,
      '--sandbox', 'read-only', '--skip-git-repo-check', '-',
    ], { env, input: fs.readFileSync(PROMPT), maxBuffer: 64 * 1024 * 1024, timeout: TIMEOUT_MS });
  } else {
    const [dir, file] = DRIVERS[REVIEWER];
    run = spawnSync('node', [
      path.join(SKILLS, dir, 'scripts', file), `@${PROMPT}`, MODELS[REVIEWER],
    ], { env, maxBuffer: 64 * 1024 * 1024, timeout: TIMEOUT_MS });
  }

  const stdout = (run.stdout || '').toString();
  const stderr = (run.stderr || '').toString();
  fs.writeFileSync(OUT, stdout);
  if (stderr) fs.writeFileSync(`${OUT}.err`, stderr);

  if (run.error && run.error.code === 'ETIMEDOUT') {
    throw new Error(
      `${REVIEWER} timed out after ${Math.round(TIMEOUT_MS / 60000)} min. A timeout is not a ` +
      'clean review — rerun it, or split the prompt.'
    );
  }
  if (run.status !== 0) {
    // Say why. A reviewer that failed must never be mistaken for one that
    // looked and found nothing — that is the worst outcome for a review gate.
    throw new Error(
      `${REVIEWER} failed (exit ${run.status}): ${stderr.trim().split('\n').slice(-2).join(' ')}`
    );
  }

  const verdict = (stdout.match(/^VEREDICTO:.*$/m) || ['(no verdict line)'])[0];
  console.error(`${REVIEWER} -> ${OUT} (${Buffer.byteLength(stdout)} bytes)`);
  console.log(verdict);
})();
