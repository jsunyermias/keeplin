// Gather everything a reviewer needs about a pull request, as files.
//
//   node collect.js <repo-path> <pr-number> <out-dir>
//
// Uses git rather than the GitHub API on purpose: api.github.com is refused by
// the egress proxy in these containers (403), while fetching refs/pull/* goes
// through the session's git proxy. It is also free in context terms — nothing
// here passes through the orchestrating agent.
//
// The PR title, body and linked issue are not reachable this way. Mirror those
// separately with the GitHub MCP tools into meta.md in the same directory; that
// is the one part of the pipeline that costs context, and only once per PR.
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const [REPO, PR, OUT] = process.argv.slice(2);

(() => {
  if (!REPO || !PR || !OUT) {
    throw new Error('usage: node collect.js <repo-path> <pr-number> <out-dir>');
  }
  if (!/^\d+$/.test(PR)) throw new Error(`pull request number must be numeric, got "${PR}"`);

  const git = (args, opts = {}) =>
    execFileSync('git', args, { cwd: REPO, maxBuffer: 256 * 1024 * 1024, ...opts });
  const gitText = (args) => git(args, { encoding: 'utf8' });

  fs.mkdirSync(path.join(OUT, 'files'), { recursive: true });

  let base = 'main';
  try {
    base = gitText(['symbolic-ref', '--quiet', '--short', 'refs/remotes/origin/HEAD'])
      .trim().replace(/^origin\//, '') || 'main';
  } catch {
    /* repositories without origin/HEAD keep the default */
  }

  const headRef = `refs/remotes/origin/pr/${PR}`;
  git(['fetch', '--quiet', 'origin', `refs/pull/${PR}/head:${headRef}`, base]);

  const mergeBase = gitText(['merge-base', `origin/${base}`, headRef]).trim();
  const range = `${mergeBase}..${headRef}`;

  fs.writeFileSync(path.join(OUT, 'diff.patch'), git(['diff', range]));
  const changed = gitText(['diff', '--name-only', range]).split('\n').filter(Boolean);
  fs.writeFileSync(path.join(OUT, 'changed-files.txt'), changed.join('\n') + '\n');

  // Reviewers reason better with the whole post-change file than with hunks
  // alone — but only for the files this PR touches. That is the "minimum
  // necessary context" the pipeline is built around, not the whole tree.
  let written = 0;
  for (const f of changed) {
    try {
      const body = git(['show', `${headRef}:${f}`]);
      fs.mkdirSync(path.join(OUT, 'files', path.dirname(f)), { recursive: true });
      fs.writeFileSync(path.join(OUT, 'files', f), body);
      written += 1;
    } catch {
      /* deleted by this PR; the diff is the only record and that is correct */
    }
  }

  const info = [
    `pr: ${PR}`,
    `base: ${base}`,
    `merge_base: ${mergeBase}`,
    `head: ${gitText(['rev-parse', headRef]).trim()}`,
    `changed_files: ${changed.length}`,
    `files_captured: ${written}`,
    `diff_bytes: ${fs.statSync(path.join(OUT, 'diff.patch')).size}`,
  ].join('\n');
  fs.writeFileSync(path.join(OUT, 'collect.info'), info + '\n');
  console.error(info);
})();
