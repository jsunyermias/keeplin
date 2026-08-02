// Contract tests for the review pipeline.
//
//   node --test scripts/tests/
//
// These cover the guarantees the pipeline is worth having for: that the gate
// cannot be talked into clearing a blocked change, that the adjudicator never
// judges its own implementation, and that nothing writes altered content while
// reporting success. Everything here runs offline — no browser, no model.
const test = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');

const SCRIPTS = path.dirname(__dirname);
const PHRASE = 'REVISION-COMPLETADA-SIN-BLOQUEANTES';

function run(script, args, opts = {}) {
  try {
    const stdout = execFileSync('node', [path.join(SCRIPTS, script), ...args], {
      encoding: 'utf8', ...opts,
    });
    return { code: 0, stdout, stderr: '' };
  } catch (e) {
    return {
      code: e.status === undefined ? 1 : e.status,
      stdout: (e.stdout || '').toString(),
      stderr: (e.stderr || '').toString(),
    };
  }
}

function tmpFile(name, body) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-test-'));
  const p = path.join(dir, name);
  fs.writeFileSync(p, body);
  return p;
}

test('gate clears only when both signals agree', () => {
  const ok = tmpFile('r.md', `VEREDICTO: SIN HALLAZGOS\n\nTodo correcto.\n\n${PHRASE}\n`);
  const r = run('gate.js', [ok]);
  assert.strictEqual(r.code, 0);
  assert.strictEqual(JSON.parse(r.stdout).cleared, true);
});

test('gate refuses a blocking verdict even when the phrase is present', () => {
  // This is the real case that occurred: the diff contains the phrase, so the
  // adjudicator reproduced it while blocking.
  const f = tmpFile('r.md', `VEREDICTO: BLOQUEANTE\n\nHay problemas.\n\n${PHRASE}\n`);
  const r = run('gate.js', [f]);
  assert.strictEqual(r.code, 1);
  assert.strictEqual(JSON.parse(r.stdout).cleared, false);
});

test('gate treats a quoted phrase as a quotation, not a declaration', () => {
  const f = tmpFile('r.md',
    `VEREDICTO: SIN HALLAZGOS\n\nEl gate exige ${PHRASE} al final.\n\nFin del análisis.\n`);
  const r = run('gate.js', [f]);
  assert.strictEqual(r.code, 1);
  const out = JSON.parse(r.stdout);
  assert.strictEqual(out.cleared, false);
  assert.strictEqual(out.phrase_quoted_not_declared, true);
});

test('gate refuses when the phrase is repeated', () => {
  const f = tmpFile('r.md',
    `VEREDICTO: SIN HALLAZGOS\n\nCito ${PHRASE} aquí.\n\n${PHRASE}\n`);
  assert.strictEqual(run('gate.js', [f]).code, 1);
});

test('gate reports a missing verdict as a failed review, not a clean one', () => {
  const f = tmpFile('r.md', 'el revisor se cayó a media respuesta\n');
  const r = run('gate.js', [f]);
  assert.strictEqual(r.code, 1);
  assert.strictEqual(JSON.parse(r.stdout).verdict, null);
  assert.match(r.stderr, /not the same as a clean review/);
});

test('gate stops at the cycle cap with a distinct exit code', () => {
  const f = tmpFile('r.md', 'VEREDICTO: BLOQUEANTE\n');
  assert.strictEqual(run('gate.js', [f, '--cycle', '5', '--max-cycles', '5']).code, 2);
});

test('the adjudicator is never the implementer', () => {
  // Each case gets its own directory: roles.js now persists its assignment, so
  // sharing one would carry a decision from a previous case into the next.
  for (const pr of [1, 2, 197, 198]) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-roles-'));
    const roles = JSON.parse(run('roles.js', [String(pr), '--dir', dir]).stdout);
    assert.notStrictEqual(roles.adjudicator, roles.implementer);
    assert.ok(!roles.independent.includes(roles.implementer));
    assert.ok(roles.reviewers.includes(roles.adjudicator));
  }
});

test('the implementer cannot be reassigned once the review has started', () => {
  // Running the same command twice only proves determinism. The rule is that a
  // later cycle cannot name a different implementer — which would promote the
  // previous one to adjudicator over a diff containing its own work.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-roles-'));
  const first = JSON.parse(run('roles.js', ['197', '--dir', dir]).stdout);
  const other = first.implementer === 'kimi' ? 'codex' : 'kimi';

  const retry = run('roles.js', ['197', other, '--dir', dir]);
  assert.strictEqual(retry.code, 1, 'reassignment must fail');
  assert.match(retry.stderr, /refusing to reassign/);

  // And the stored assignment is unchanged.
  const again = JSON.parse(run('roles.js', ['197', '--dir', dir]).stdout);
  assert.strictEqual(again.implementer, first.implementer);
  assert.strictEqual(again.adjudicator, first.adjudicator);
});

test('gate refuses a clean review that came from the implementer', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-roles-'));
  const roles = JSON.parse(run('roles.js', ['197', '--dir', dir]).stdout);
  const review = path.join(dir, 'review.md');
  fs.writeFileSync(review, `VEREDICTO: SIN HALLAZGOS\n\nTodo bien.\n\n${PHRASE}\n`);
  // Attributed to the implementer rather than the adjudicator.
  fs.writeFileSync(`${review}.meta.json`, JSON.stringify({ reviewer: roles.implementer }));

  const r = run('gate.js', [review, '--roles', path.join(dir, 'roles-pr197.json')]);
  assert.strictEqual(r.code, 2);
  assert.match(r.stderr, /that is the implementer/);
});

test('gate refuses a review with no recorded reviewer', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-roles-'));
  run('roles.js', ['197', '--dir', dir]);
  const review = path.join(dir, 'review.md');
  fs.writeFileSync(review, `VEREDICTO: SIN HALLAZGOS\n\n${PHRASE}\n`);

  const r = run('gate.js', [review, '--roles', path.join(dir, 'roles-pr197.json')]);
  assert.strictEqual(r.code, 2);
  assert.match(r.stderr, /cannot be established/);
});

test('apply-files refuses to write through a symlink leaving the repository', () => {
  // path.resolve is textual: a link inside the repository pointing outside
  // passes it, and writeFileSync follows the link.
  const outside = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-outside-'));
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-repo-'));
  fs.symlinkSync(outside, path.join(repo, 'enlace'));
  const reply = tmpFile('reply.md', 'FICHERO: enlace/robado.js\n```\nx\n```\n');

  const r = run('apply-files.js', [repo, reply]);
  assert.strictEqual(r.code, 1);
  assert.match(r.stderr, /symlink that leaves the repository/);
  assert.ok(!fs.existsSync(path.join(outside, 'robado.js')), 'nothing written outside');
});

test('roles alternate across pull requests', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-roles-'));
  const odd = JSON.parse(run('roles.js', ['197', '--dir', dir]).stdout);
  const even = JSON.parse(run('roles.js', ['198', '--dir', dir]).stdout);
  assert.notStrictEqual(odd.implementer, even.implementer);
});

test('apply-files keeps a markdown file\'s inner fences intact', () => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-repo-'));
  const reply = tmpFile('reply.md', [
    'FICHERO: README.md',
    '```',
    '# Título',
    '',
    'Ejemplo:',
    '',
    '```js',
    'const a = 1;',
    '```',
    '',
    'Fin.',
    '```',
  ].join('\n'));
  const r = run('apply-files.js', [repo, reply]);
  assert.strictEqual(r.code, 0, r.stderr);
  const written = fs.readFileSync(path.join(repo, 'README.md'), 'utf8');
  assert.match(written, /```js/);
  assert.match(written, /const a = 1;/);
  assert.match(written, /Fin\./);
});

test('apply-files refuses to write outside the repository', () => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-repo-'));
  const reply = tmpFile('reply.md', 'FICHERO: ../escapado.js\n```\nx\n```\n');
  const r = run('apply-files.js', [repo, reply]);
  assert.strictEqual(r.code, 1);
  assert.match(r.stderr, /outside the repository/);
});

test('apply-files reports a reply that ignored the contract', () => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-repo-'));
  const reply = tmpFile('reply.md', 'Claro, aquí tienes el cambio explicado en prosa.\n');
  const r = run('apply-files.js', [repo, reply]);
  assert.strictEqual(r.code, 1);
  assert.match(r.stderr, /did not follow the contract/);
});

function gitRepo(files) {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-repo-'));
  execFileSync('git', ['init', '-q'], { cwd: repo });
  execFileSync('git', ['config', 'user.email', 'a@b.c'], { cwd: repo });
  execFileSync('git', ['config', 'user.name', 't'], { cwd: repo });
  for (const [name, body] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(repo, name)), { recursive: true });
    fs.writeFileSync(path.join(repo, name), body);
  }
  execFileSync('git', ['add', '-A'], { cwd: repo });
  execFileSync('git', ['commit', '-qm', 'init'], { cwd: repo });
  return repo;
}

test('apply-patch applies a diff whose @@ header the model miscounted', () => {
  const repo = gitRepo({
    'calc.js': 'function suma(a, b) {\n  return a + b;\n}\n\nmodule.exports = { suma };\n',
  });
  const reply = tmpFile('reply.md', [
    '--- reply ---',
    'diff --git a/calc.js b/calc.js',
    '--- a/calc.js',
    '+++ b/calc.js',
    '@@ -1,99 +1,99 @@',                 // deliberately wrong counts
    ' function suma(a, b) {',
    '   return a + b;',
    ' }',
    '',
    '-module.exports = { suma };',
    '+function resta(a, b) {',
    '+  return a - b;',
    '+}',
    '+',
    '+module.exports = { suma, resta };',
    'Ask anything, or task an agent...',       // UI chrome that must be trimmed
  ].join('\n'));

  const r = run('apply-patch.js', [repo, reply]);
  assert.strictEqual(r.code, 0, r.stderr);
  const out = fs.readFileSync(path.join(repo, 'calc.js'), 'utf8');
  assert.match(out, /function resta/);
  assert.match(out, /\}\n\nfunction resta/);   // the blank line survives
});

test('apply-patch refuses, and writes nothing, when a context line was lost', () => {
  // The real damage a rendered chat does: the context line is deleted, not
  // blanked. No flag recovers that, so the only safe outcome is a loud refusal
  // rather than a partly-applied file.
  const repo = gitRepo({
    'calc.js': 'function suma(a, b) {\n  return a + b;\n}\n\nmodule.exports = { suma };\n',
  });
  const before = fs.readFileSync(path.join(repo, 'calc.js'), 'utf8');
  const reply = tmpFile('reply.md', [
    'diff --git a/calc.js b/calc.js',
    '--- a/calc.js',
    '+++ b/calc.js',
    '@@ -1,5 +1,7 @@',
    ' function suma(a, b) {',
    '   return a + b;',
    ' }',
    // the blank context line belongs here and the renderer removed it
    '-module.exports = { suma };',
    '+module.exports = { suma, resta };',
  ].join('\n'));

  const r = run('apply-patch.js', [repo, reply]);
  assert.strictEqual(r.code, 1);
  assert.match(r.stderr, /does not apply cleanly/);
  assert.strictEqual(fs.readFileSync(path.join(repo, 'calc.js'), 'utf8'), before);
});

test('apply-patch applies a patch that creates a file', () => {
  // The extended header on line two used to cut the patch off right there, and
  // git blamed the implementer for a patch that was correct.
  const repo = gitRepo({ 'existente.txt': 'x\n' });
  const reply = tmpFile('reply.md', [
    '--- reply ---',
    'diff --git a/nuevo.js b/nuevo.js',
    'new file mode 100644',
    'index 0000000..a1b2c3d',
    '--- /dev/null',
    '+++ b/nuevo.js',
    '@@ -0,0 +1,3 @@',
    '+function resta(a, b) {',
    '+  return a - b;',
    '+}',
    'Ask anything, or task an agent...',
  ].join('\n'));

  const r = run('apply-patch.js', [repo, reply]);
  assert.strictEqual(r.code, 0, r.stderr);
  assert.match(fs.readFileSync(path.join(repo, 'nuevo.js'), 'utf8'), /function resta/);
});

test('apply-patch applies a patch that deletes and renames', () => {
  const repo = gitRepo({ 'viejo.txt': 'contenido\n', 'sobra.txt': 'basura\n' });
  const reply = tmpFile('reply.md', [
    'diff --git a/sobra.txt b/sobra.txt',
    'deleted file mode 100644',
    'index 1111111..0000000',
    '--- a/sobra.txt',
    '+++ /dev/null',
    '@@ -1 +0,0 @@',
    '-basura',
    'diff --git a/viejo.txt b/nuevo.txt',
    'similarity index 100%',
    'rename from viejo.txt',
    'rename to nuevo.txt',
  ].join('\n'));

  const r = run('apply-patch.js', [repo, reply]);
  assert.strictEqual(r.code, 0, r.stderr);
  assert.ok(!fs.existsSync(path.join(repo, 'sobra.txt')), 'deletion applied');
  assert.ok(fs.existsSync(path.join(repo, 'nuevo.txt')), 'rename applied');
});

test('build-prompt keeps the required contract whatever the budget', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-bp-'));
  fs.mkdirSync(path.join(dir, 'context'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'diff.patch'), 'diff --git a/x b/x\n');
  fs.writeFileSync(path.join(dir, 'changed-files.txt'), '');
  // Deliberately larger than CONTEXT_BUDGET_BYTES: it must still be carried.
  fs.writeFileSync(path.join(dir, 'context', 'AGENTS.md'),
    '# contrato\n' + 'x'.repeat(70 * 1024));
  const checklist = tmpFile('c.md', '# checklist\n');

  const r = run('build-prompt.js', [dir, checklist]);
  assert.strictEqual(r.code, 0, r.stderr);
  const prompt = fs.readFileSync(path.join(dir, 'prompt.txt'), 'utf8');
  assert.match(prompt, /# contrato/);
});

test('apply-patch refuses prose instead of guessing', () => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-repo-'));
  const reply = tmpFile('reply.md', 'He añadido la función resta, avísame si quieres tests.\n');
  const r = run('apply-patch.js', [repo, reply]);
  assert.strictEqual(r.code, 1);
  assert.match(r.stderr, /no unified diff found/);
});

// A repository whose fetch of refs/pull/<n>/head actually succeeds, so the
// AGENTS.md check is reached. The previous version of this test used a repo
// with no remote: git failed first with "fatal", the assertion matched that,
// and the test would have kept passing with the check deleted.
function repoWithPullRef(files) {
  const origin = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-origin-'));
  execFileSync('git', ['init', '-q', '--bare'], { cwd: origin });

  const work = gitRepo(files);
  execFileSync('git', ['remote', 'add', 'origin', origin], { cwd: work });
  execFileSync('git', ['push', '-q', 'origin', 'HEAD:refs/heads/main'], { cwd: work });
  // Give the base a second commit so merge-base has something to resolve to.
  fs.writeFileSync(path.join(work, 'later.txt'), 'x\n');
  execFileSync('git', ['add', '-A'], { cwd: work });
  execFileSync('git', ['commit', '-qm', 'change'], { cwd: work });
  execFileSync('git', ['push', '-q', 'origin', 'HEAD:refs/pull/1/head'], { cwd: work });
  return work;
}

test('collect refuses when the project contract is absent', () => {
  const repo = repoWithPullRef({ 'src.js': 'const a = 1;\n' });
  const out = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-out-'));
  const r = run('collect.js', [repo, '1', out]);
  assert.strictEqual(r.code, 1);
  // Specifically the contract check, not git failing for some other reason.
  assert.match(r.stderr, /required project context missing/);
});

test('collect refreshes the base before computing the merge base', () => {
  // The base advances behind this clone's back, so a run that failed to
  // refresh it would compute the merge base against a stale commit. Note this
  // passes with either fetch form: the claim that a bare branch name leaves
  // the tracking ref stale did not survive measurement. It is kept as a
  // regression guard on the outcome, not as evidence for that claim.
  const repo = repoWithPullRef({
    'src.js': 'const a = 1;\n',
    'AGENTS.md': '# contrato\n',
  });
  const origin = execFileSync('git', ['remote', 'get-url', 'origin'],
    { cwd: repo, encoding: 'utf8' }).trim();

  // Advance main in the origin, behind this clone's back.
  const other = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-other-'));
  execFileSync('git', ['clone', '-q', origin, other]);
  // The bare origin also carries refs/pull/1/head, so the clone's HEAD is not
  // necessarily main; land on it explicitly before advancing it.
  execFileSync('git', ['checkout', '-q', '-B', 'main', 'origin/main'], { cwd: other });
  execFileSync('git', ['config', 'user.email', 'a@b.c'], { cwd: other });
  execFileSync('git', ['config', 'user.name', 't'], { cwd: other });
  fs.writeFileSync(path.join(other, 'avance.txt'), 'nuevo\n');
  execFileSync('git', ['add', '-A'], { cwd: other });
  execFileSync('git', ['commit', '-qm', 'advance main'], { cwd: other });
  execFileSync('git', ['push', '-q', 'origin', 'HEAD:refs/heads/main'], { cwd: other });

  const out = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-out-'));
  const r = run('collect.js', [repo, '1', out]);
  assert.strictEqual(r.code, 0, r.stderr);

  const info = fs.readFileSync(path.join(out, 'collect.info'), 'utf8');
  const mergeBase = (info.match(/^merge_base: (\w+)$/m) || [])[1];
  const originMain = execFileSync('git', ['rev-parse', 'refs/remotes/origin/main'],
    { cwd: repo, encoding: 'utf8' }).trim();
  const advanced = execFileSync('git', ['rev-parse', 'HEAD'],
    { cwd: other, encoding: 'utf8' }).trim();

  // The tracking ref must have caught up with the advanced base.
  assert.strictEqual(originMain, advanced, 'origin/main was not refreshed');
  assert.ok(mergeBase, 'a merge base was computed');
});

test('collect carries the project contract when it is present', () => {
  const repo = repoWithPullRef({
    'src.js': 'const a = 1;\n',
    'AGENTS.md': '# contrato del proyecto\n',
  });
  const out = fs.mkdtempSync(path.join(os.tmpdir(), 'mmr-out-'));
  const r = run('collect.js', [repo, '1', out]);
  assert.strictEqual(r.code, 0, r.stderr);
  assert.ok(fs.existsSync(path.join(out, 'context', 'AGENTS.md')));
  assert.match(fs.readFileSync(path.join(out, 'collect.info'), 'utf8'), /context: AGENTS\.md/);
});
