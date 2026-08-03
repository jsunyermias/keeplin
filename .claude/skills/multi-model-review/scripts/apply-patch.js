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

const { parse } = require('./args');

const ARGS = parse(process.argv.slice(2), {
  name: 'apply-patch.js',
  positionals: ['repo-path', 'reply-file'],
  flags: { check: {} },
  usage: 'apply-patch.js <repo-path> <reply-file> [--check]',
});
const CHECK = ARGS.check;
const [REPO, REPLY] = ARGS.positional;

// A chat reply wraps the patch in a fence and pads it with prose. Take the
// largest fenced block that looks like a diff; if there is no fence, assume the
// whole reply is the patch.
// Fences are matched as whole lines, never as a substring. The old expression
// closed on the first ``` anywhere in the text, and a patch that adds a
// Markdown code block contains lines like "+```" — so the capture ended inside
// the diff. What came out was a shorter patch that --recount could still turn
// into something git would apply: a silent, partial write reported as success.
// That is worse than a refusal, so the close must be a line of its own.
function fencedBlocks(text) {
  const lines = text.split('\n');
  const blocks = [];
  let open = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (open === null) {
      if (/^\s*```(diff|patch)?\s*$/.test(line)) open = i;
    } else if (/^\s*```\s*$/.test(line)) {
      blocks.push(lines.slice(open + 1, i).join('\n'));
      open = null;
    }
  }
  return blocks;
}

function extractPatch(text) {
  const candidates = fencedBlocks(text)
    .filter((f) => /^(diff --git|--- |\+\+\+ |@@ )/m.test(f));
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
// git's extended headers appear on the second line of any patch that creates,
// deletes, renames, copies or changes the mode of a file. Omitting them cut the
// patch off right after `diff --git`, and git then reported "No valid patches
// in input" — blaming the implementer for a patch that was correct, which sends
// the cycle into re-asking for something already right.
const PATCH_LINE = new RegExp('^(' + [
  'diff --git ', 'index ',
  'new file mode ', 'deleted file mode ', 'old mode ', 'new mode ',
  'copy from ', 'copy to ', 'rename from ', 'rename to ',
  'similarity index ', 'dissimilarity index ',
  'Binary files ', 'GIT binary patch',
  '--- ', '\\+\\+\\+ ', '@@ ',
  '[ +\\-\\\\]',
].join('|') + ')');

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



(() => {
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
  // Every exit removes it, not just the successful one. A patch that fails
  // --check is the ordinary case in a review cycle, and each failure used to
  // strand a directory holding the implementer's diff.
  try {
    run(tmpDir, patch);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
})();

function run(tmpDir, patch) {
  const tmp = path.join(tmpDir, 'change.patch');
  // Patches that lose their trailing newline are rejected by git apply.
  fs.writeFileSync(tmp, patch.endsWith('\n') ? patch : `${patch}\n`);

  const git = (args) =>
    execFileSync('git', args, { cwd: REPO, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });

  // --recount forgives the miscounted @@ headers models routinely emit; -C1
  // helps when a hunk is short of context at its edges.
  //
  // Neither rescues the failure that actually occurs here. A renderer does not
  // blank a context line, it deletes it, and a hunk missing an interior context
  // line is rejected by git whatever the flags — measured, not assumed. There
  // was a repair() here that converted empty lines back to spaces; it was built
  // on the wrong theory, git accepts such a patch unaided, and removing it
  // changed no behaviour. When this refuses, re-ask the implementer.
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

  console.error(stat || '(patch applied, no net change)');
}
