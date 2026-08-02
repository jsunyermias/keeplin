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

// --area may be repeated. One prefix cannot always express the seam a review
// should be split along: this skill's own tests live in a single file beside
// the scripts they cover, so "the write path and its tests" is a set of paths,
// not a directory.
const argv = process.argv.slice(2);
const AREAS = [];
const consumed = new Set();
argv.forEach((a, i) => {
  if (a === '--area') {
    consumed.add(i);
    if (argv[i + 1] && !argv[i + 1].startsWith('--')) {
      AREAS.push(argv[i + 1]);
      consumed.add(i + 1);
    }
  }
});
const [REPO, PR, OUT] = argv.filter((a, i) => !a.startsWith('--') && !consumed.has(i));

(() => {
  if (!REPO || !PR || !OUT) {
    throw new Error(
      'usage: node collect.js <repo-path> <pr-number> <out-dir> [--area <path>]...'
    );
  }
  if (argv.includes('--area') && AREAS.length !== argv.filter((a) => a === '--area').length) {
    throw new Error('--area needs a path');
  }
  const AREA = AREAS.length ? AREAS.join(' ') : null;
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
  // The base is fetched with an explicit refspec rather than by bare name. A
  // reviewer argued the bare form leaves it in FETCH_HEAD and lets the merge
  // base go stale; measured, that is not so — git still applies the remote's
  // configured refspec and updates the tracking ref. The explicit form is kept
  // because it does not depend on the remote being configured that way, not
  // because the bare one was broken.
  git(['fetch', '--quiet', 'origin',
    `refs/pull/${PR}/head:${headRef}`,
    `refs/heads/${base}:refs/remotes/origin/${base}`,
  ]);

  // GitHub updates refs/pull/<n>/head a little after the branch push, so a
  // collect run straight after pushing fetches the previous head and every
  // reviewer downstream judges code that is not what was just written. Nothing
  // fails; the package is internally consistent and simply describes an older
  // commit. Say so, loudly, rather than leaving it to whoever thinks to read
  // the hash in collect.info.
  const prHead = gitText(['rev-parse', headRef]).trim();
  let behind = false;
  try {
    const local = gitText(['rev-parse', 'HEAD']).trim();
    behind = local !== prHead
      && execFileSync('git', ['merge-base', '--is-ancestor', prHead, local],
        { cwd: REPO }) !== null;
  } catch {
    /* the checkout is on some unrelated commit; nothing useful to compare */
  }

  const mergeBase = gitText(['merge-base', `origin/${base}`, headRef]).trim();
  const range = `${mergeBase}..${headRef}`;

  // A pull request can outgrow what a browser chat will take — 400 KB of diff
  // does not get reviewed, it gets skimmed. The documentation already said to
  // split such a change by area and run the rotation per area, but there was no
  // way to do it, so the split happened by hand and one such hand-made package
  // reached the reviewers with no whole-file context at all. --area is that
  // missing mechanism: everything downstream still sees an ordinary package,
  // and the scope is recorded in collect.info so a reader knows what was and
  // was not looked at.
  const scope = AREAS.length ? ['--', ...AREAS] : [];

  fs.writeFileSync(path.join(OUT, 'diff.patch'), git(['diff', range, ...scope]));
  const changed = gitText(['diff', '--name-only', range, ...scope]).split('\n').filter(Boolean);
  if (!changed.length) {
    throw new Error(
      AREA
        ? `no files under "${AREA}" changed between ${mergeBase} and the pull request head`
        : `no files changed between ${mergeBase} and the pull request head`
    );
  }
  fs.writeFileSync(path.join(OUT, 'changed-files.txt'), changed.join('\n') + '\n');

  // Reviewers reason better with the whole post-change file than with hunks
  // alone — but only for the files this PR touches. That is the "minimum
  // necessary context" the pipeline is built around, not the whole tree.
  // Binary files come back as replacement characters once read as UTF-8: pure
  // noise that costs tokens and tells a reviewer nothing. git reports them as
  // "-\t-" in numstat, so leave them to the diff alone.
  const binary = new Set(
    gitText(['diff', '--numstat', range, ...scope]).split('\n')
      .filter((l) => l.startsWith('-\t-\t'))
      .map((l) => l.split('\t')[2])
      .filter(Boolean)
  );

  let written = 0;
  for (const f of changed) {
    if (binary.has(f)) continue;
    try {
      const body = git(['show', `${headRef}:${f}`]);
      fs.mkdirSync(path.join(OUT, 'files', path.dirname(f)), { recursive: true });
      fs.writeFileSync(path.join(OUT, 'files', f), body);
      written += 1;
    } catch {
      /* deleted by this PR; the diff is the only record and that is correct */
    }
  }

  // Reviewers cannot open the repository, yet the checklist orders them to read
  // AGENTS.md and the applicable companions. Shipping the diff without the
  // contract asks for a judgement they have no basis to make, so the required
  // resources travel with it and their absence is recorded rather than hidden.
  const REQUIRED = ['AGENTS.md'];
  const OPTIONAL = ['docs/review-debt.md', '.github/pull_request_template.md'];
  fs.mkdirSync(path.join(OUT, 'context'), { recursive: true });
  const carried = [];
  const missing = [];
  for (const rel of [...REQUIRED, ...OPTIONAL]) {
    const from = path.join(REPO, rel);
    if (fs.existsSync(from)) {
      const to = path.join(OUT, 'context', rel.replace(/[\\/]/g, '__'));
      fs.copyFileSync(from, to);
      carried.push(rel);
    } else if (REQUIRED.includes(rel)) {
      missing.push(rel);
    }
  }
  if (missing.length) {
    throw new Error(
      `required project context missing from ${REPO}: ${missing.join(', ')}. ` +
      'A review without it cannot check what the checklist asks for.'
    );
  }

  const info = [
    `pr: ${PR}`,
    `area: ${AREA || '(whole pull request)'}`,
    `base: ${base}`,
    `merge_base: ${mergeBase}`,
    `head: ${gitText(['rev-parse', headRef]).trim()}`,
    `changed_files: ${changed.length}`,
    `files_captured: ${written}`,
    `binary_skipped: ${binary.size}`,
    `diff_bytes: ${fs.statSync(path.join(OUT, 'diff.patch')).size}`,
    `context: ${carried.join(', ')}`,
  ].join('\n');
  fs.writeFileSync(path.join(OUT, 'collect.info'), info + '\n');
  console.error(info);
  if (behind) {
    console.error(
      `\nwarning: refs/pull/${PR}/head is behind this checkout — the pull request ref has not ` +
      'caught up with the branch yet. This package describes the older commit, so the review ' +
      'would be of code that is not what you just pushed. Wait and rerun.'
    );
  }
})();
