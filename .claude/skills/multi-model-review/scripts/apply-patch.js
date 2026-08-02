// Apply an implementer's patch to a repository without reading it.
//
//   node apply-patch.js <repo-path> <reply-file> [--check]
//
// Kimi is a chat: it can write code but cannot push a branch, so it is asked
// for a unified diff. Extracting and applying that diff mechanically is what
// keeps the implementation out of the orchestrator's context — `git apply`
// needs the bytes, not a reader. Codex edits the tree directly and does not go
// through here.
//
// Reports only what changed, never the contents.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { execFileSync } = require('child_process');

const argv = process.argv.slice(2);
const CHECK = argv.includes('--check');
const [REPO, REPLY] = argv.filter((a) => !a.startsWith('--'));

// A chat reply wraps the patch in a fence and pads it with prose. Take the
// largest fenced block that looks like a diff; if there is no fence, assume the
// whole reply is the patch.
function extractPatch(text) {
  const fences = [...text.matchAll(/```(?:diff|patch)?\n([\s\S]*?)```/g)].map((m) => m[1]);
  const candidates = fences.filter((f) => /^(diff --git|--- |\+\+\+ |@@ )/m.test(f));
  if (candidates.length) return trimToPatch(candidates.sort((a, b) => b.length - a.length)[0]);
  if (/^(diff --git|@@ )/m.test(text)) return trimToPatch(text);
  return null;
}

// Rendered chat output has no fences to cut on, so the reply arrives wrapped in
// UI chrome: reasoning above, the composer's placeholder below. git apply
// rejects the whole patch over one stray line, so bound it by what actually
// looks like patch content.
// Must not match the driver's own '--- reply ---' banner, so the file-header
// form is required rather than a bare '--- '.
const PATCH_START = /^(diff --git |--- [ab]\/)/;
const PATCH_LINE = /^(diff --git |index |--- |\+\+\+ |@@ |[ +\-\\])/;

function trimToPatch(block) {
  const lines = block.split('\n');
  const first = lines.findIndex((l) => PATCH_START.test(l));
  if (first === -1) return block;
  let last = first;
  for (let i = first; i < lines.length; i++) {
    if (PATCH_LINE.test(lines[i]) || lines[i] === '') last = i;
    else break;
  }
  // Drop blank lines the chrome left hanging off the end.
  while (last > first && lines[last].trim() === '') last -= 1;
  return lines.slice(first, last + 1).join('\n');
}


// Repair the two things a rendered chat reliably destroys in a unified diff.
//
// A blank context line is a single space in the patch format, and innerText
// strips it to an empty line — git then rejects the whole patch as corrupt.
// And models routinely miscount the @@ header, which `git apply --recount`
// exists to forgive. Both are mechanical; neither requires reading the change.
function repair(patch) {
  const lines = patch.split('\n');
  let inHunk = false;
  const out = lines.map((line, idx) => {
    if (line.startsWith('@@')) { inHunk = true; return line; }
    if (/^(diff --git|index |--- |\+\+\+ )/.test(line)) { inHunk = false; return line; }
    // Only inside a hunk, and never the trailing newline at the very end.
    if (inHunk && line === '' && idx !== lines.length - 1) return ' ';
    return line;
  });
  return out.join('\n');
}

(() => {
  if (!REPO || !REPLY) {
    throw new Error('usage: node apply-patch.js <repo-path> <reply-file> [--check]');
  }

  const patch = extractPatch(fs.readFileSync(REPLY, 'utf8'));
  if (!patch) {
    throw new Error(
      `no unified diff found in ${REPLY} — the implementer answered in prose. ` +
      'Re-ask for a patch; do not hand-transcribe the reply.'
    );
  }

  // A predictable /tmp path that writeFileSync follows through a symlink could
  // clobber another file owned by this user. Own the directory instead.
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'impl-'));
  const tmp = path.join(tmpDir, 'change.patch');
  const repaired = repair(patch);
  // Patches that lose their trailing newline are rejected by git apply.
  fs.writeFileSync(tmp, repaired.endsWith('\n') ? repaired : `${repaired}\n`);

  const git = (args) =>
    execFileSync('git', args, { cwd: REPO, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });

  // Rendering can delete a blank context line outright rather than blanking it,
  // and then there is nothing left to repair — the hunk simply has fewer
  // context lines than the file. Falling back to a single line of required
  // context recovers those without loosening what the patch actually changes.
  const attempts = [
    ['--recount'],
    ['--recount', '-C1'],
  ];
  let mode = null;
  let lastErr = '';
  for (const extra of attempts) {
    try {
      git(['apply', '--check', ...extra, tmp]);
      mode = extra;
      break;
    } catch (e) {
      lastErr = String(e.stderr || e.message).trim().split('\n').filter(Boolean).pop() || '';
    }
  }
  if (!mode) {
    throw new Error(
      `patch does not apply cleanly: ${lastErr}. ` +
      'Send the reviewer feedback back to the implementer rather than editing by hand.'
    );
  }
  if (mode.includes('-C1')) {
    console.error('note: applied with reduced context — the rendered reply lost context lines');
  }

  if (CHECK) {
    console.error(`patch applies cleanly (${Buffer.byteLength(patch)} bytes), not applied`);
    return;
  }

  // --3way lets git fall back to a merge when context has drifted, which is
  // common once a review cycle has already changed the branch.
  git(['apply', '--3way', ...mode, tmp]);
  const stat = git(['diff', '--stat']).trim();
  fs.rmSync(tmpDir, { recursive: true, force: true });

  console.error(stat || '(patch applied, no net change)');
})();
