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

const { parse } = require('./args');

const ARGS = parse(process.argv.slice(2), {
  name: 'ask.js',
  positionals: ['reviewer', 'prompt-file', 'out-file'],
  options: { pr: { integer: true } },
  usage: 'ask.js <qwen|glm|kimi|codex> <prompt-file> <out-file> [--pr N]',
});
const [REVIEWER, PROMPT, OUT] = ARGS.positional;
const PR = ARGS.pr;

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

// Input ceilings, in characters. Only what has actually been measured belongs
// here: Qwen refuses above this with "si necesita superar los 131072 caracteres
// de entrada de texto...", which costs ten minutes of driving a browser to
// discover. A family whose limit is unknown is absent rather than guessed — a
// wrong number here would refuse work that would have succeeded, and inventing
// one to look complete is the kind of confident-but-unfounded claim this
// pipeline exists to catch.
const INPUT_LIMITS = {
  qwen: 131072,
};

const DRIVERS = {
  qwen: ['qwen-web-chat', 'qwen.js'],
  glm: ['glm-web-chat', 'glm.js'],
  kimi: ['kimi-web-chat', 'kimi.js'],
};

(() => {
  if (!fs.existsSync(PROMPT)) throw new Error(`no such prompt file: ${PROMPT}`);
  if (!MODELS[REVIEWER]) throw new Error(`unknown reviewer: ${REVIEWER}`);

  // Clear any previous review at this path before running. Otherwise a rerun
  // that fails leaves the earlier run's file and .meta.json in place, and both
  // still look valid: a clean review of an older state of the same pull
  // request, ready for the gate to close a cycle on code that has since
  // changed. Removing them first means a failed run leaves no review at all,
  // which is the truth.
  for (const stale of [OUT, `${OUT}.meta.json`, `${OUT}.err`]) {
    if (fs.existsSync(stale)) fs.rmSync(stale);
  }

  // Fail in a second rather than after ten minutes of driving a browser to a
  // refusal. The prompt is unchanged and can be rebuilt smaller — with
  // --no-files, or split with collect.js --area — and rerun.
  const limit = INPUT_LIMITS[REVIEWER];
  const size = fs.readFileSync(PROMPT, 'utf8').length;
  if (limit && size > limit) {
    throw new Error(
      `${PROMPT} is ${size} characters and ${REVIEWER} refuses anything over ${limit}. ` +
      'Rebuild it smaller (build-prompt --no-files, or collect --area) rather than sending a ' +
      'prompt that cannot be answered.'
    );
  }

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
  // Whatever came back goes to .raw for diagnosis, never straight to OUT. A
  // failed run used to leave a short file at the reviewer's own path — one
  // stall wrote 15 bytes of driver banner there — and nothing downstream
  // distinguishes that from a review. The gate is safe because it also wants
  // .meta.json, but `build-prompt --prior` would attach the banner to the
  // adjudicator's prompt as though a reviewer had said it.
  fs.writeFileSync(`${OUT}.raw`, stdout);
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

  // An empty reply is not a review, and neither is a non-empty one with no
  // verdict in it. Both mean the run failed — a stall, a truncation, a driver
  // that returned its own banner — and the worst outcome for a review gate is a
  // failure that reads like "looked, found nothing".
  // The prompt asks for one of exactly three verdicts, as the first line. Any
  // line beginning "VEREDICTO:" used to satisfy this — including the prompt's
  // own line listing the options, which these UIs echo back, and including
  // "VEREDICTO: cualquier cosa". A reply that echoes the instructions is not a
  // review, and publishing it would put it in front of the adjudicator as one.
  const VERDICTS = ['BLOQUEANTE', 'OBSERVACIONES', 'SIN HALLAZGOS'];
  // The browser drivers print their own header — model, chat url — and then a
  // "--- reply ---" marker before the model's words. Everything before that
  // marker is ours, not the reviewer's, and judging the reply by it rejected
  // every valid browser review the moment the first-line rule landed.
  const REPLY_MARKER = /^-{3}\s*reply\s*-{3}$/m;
  const whole = stdout.replace(/\r\n?/g, '\n');
  const marker = whole.match(REPLY_MARKER);
  const body = marker
    ? whole.slice(whole.indexOf(marker[0]) + marker[0].length)
    : whole;

  // The verdict must be declared before any substance, but not literally on
  // line one: these UIs prepend their own chrome — "Show full message",
  // "Thought Process" — and requiring line one rejected a real review from GLM
  // for a banner it did not write. So: skip known chrome, then the next
  // non-empty line must be the verdict. That still refuses the case this check
  // exists for, a reply whose verdict appears somewhere in the middle, and the
  // prompt's own line listing all three options.
  const CHROME = /^(show full message|thought process|thinking|pensando|copiar|copy)$/i;
  const meaningful = body.split('\n')
    .map((l) => l.trim())
    .filter((l) => l !== '' && !CHROME.test(l));
  const first = meaningful[0] || '';
  // The verdict must open the line, but reviewers routinely append a summary to
  // it — "VEREDICTO: BLOQUEANTE - SKILL.md línea 174 ...". Refusing that threw
  // away a real Qwen review over punctuation. So match the verdict as a prefix,
  // and refuse a line that declares more than one: the prompt's own line
  // listing the three options is exactly that shape, and an echoed prompt must
  // never read as a verdict.
  const declared = (first.match(
    new RegExp(`^VEREDICTO:\\s*(${VERDICTS.join('|')})\\b`, 'i')) || [])[1];
  const restated = (first.match(/VEREDICTO:/gi) || []).length;
  const verdictLine = declared && restated === 1
    ? `VEREDICTO: ${declared.toUpperCase()}`
    : null;

  if (!verdictLine) {
    throw new Error(
      `${REVIEWER} did not open with one of ${VERDICTS.join(' | ')} ` +
      `(${Buffer.byteLength(stdout)} bytes, first line: ${JSON.stringify(first.slice(0, 80))}). ` +
      `That is a failed run, not a clean review — the reply is in ${OUT}.raw. ` +
      'Rerun it, or split the prompt.'
    );
  }

  // Only now does the file appear at the reviewer's path, so anything that
  // finds one can rely on it being a real review.
  fs.writeFileSync(OUT, stdout);

  // Record who produced this file, and for which pull request. Without the
  // first the gate can only trust that whoever ran the command typed the right
  // reviewer name; without the second a roles file from another pull request
  // with the same adjudicator would pass the independence check.
  fs.writeFileSync(`${OUT}.meta.json`, JSON.stringify({
    reviewer: REVIEWER,
    model: MODELS[REVIEWER],
    ...(PR === undefined ? {} : { pr: Number(PR) }),
    at: new Date().toISOString(),
  }, null, 2) + '\n');

  console.error(`${REVIEWER} -> ${OUT} (${Buffer.byteLength(stdout)} bytes)`);
  console.log(verdictLine);
})();
