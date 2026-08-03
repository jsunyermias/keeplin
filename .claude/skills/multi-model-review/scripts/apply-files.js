// Write whole files an implementer produced, without reading them.
//
//   node apply-files.js <repo-path> <reply-file> [--check]
//
// Chat implementers are asked for complete file contents, not a unified diff.
// A diff cannot survive the round trip: the page renderer deletes blank context
// lines outright rather than blanking them, and a hunk missing a context line
// is rejected by git with no way to repair it mechanically — reducing required
// context does not recover it either. A whole file has no context lines to
// lose, so the same rendering damage is at worst cosmetic.
//
// Expected shape in the reply, repeated per file:
//
//   FICHERO: ruta/relativa.js
//   ```
//   ...contenido completo...
//   ```
const fs = require('fs');
const path = require('path');

const { parse } = require('./args');

const ARGS = parse(process.argv.slice(2), {
  name: 'apply-files.js',
  positionals: ['repo-path', 'reply-file'],
  flags: { check: {} },
  usage: 'apply-files.js <repo-path> <reply-file> [--check]',
});
const CHECK = ARGS.check;
const [REPO, REPLY] = ARGS.positional;

const HEADER = /^\s*(?:FICHERO|FILE|ARCHIVO)\s*:\s*(\S+)\s*$/i;

// The reply arrives as rendered text, so fences may or may not survive. Take
// everything between one header and the next as that file's body.
//
// Only the block's own outer fences are removed — the opening one just after
// the header and its matching close. Stripping every line that starts with a
// fence would quietly gut a Markdown file's inner code blocks, turning an
// apparently successful write into altered content, which is worse than a
// visible failure.
// Language labels a chat UI puts above a fenced block. A closed list, not a
// shape: `/^[a-z]{1,12}$/` matched any short lowercase word, so a file whose
// first line was `import`, `export`, `package` or `use` lost it — silently, and
// the loop would take several such lines in a row. Deleting real content while
// reporting a successful write is the worst thing this file can do.
const LANGUAGE_LABELS = new Set([
  'js', 'jsx', 'ts', 'tsx', 'javascript', 'typescript', 'json', 'jsonc',
  'sh', 'bash', 'zsh', 'shell', 'console', 'diff', 'patch', 'md', 'markdown',
  'yaml', 'yml', 'toml', 'ini', 'sql', 'html', 'css', 'scss', 'xml',
  'py', 'python', 'rb', 'ruby', 'go', 'rs', 'rust', 'java', 'kt', 'c', 'cpp',
  'text', 'plaintext', 'txt', 'none',
]);
const COPY_AFFORDANCE = /^(copy|copiar|copy code|copiar código)$/i;

function parseReply(text) {
  const lines = text.split('\n');
  const marks = [];
  lines.forEach((l, i) => {
    const m = l.match(HEADER);
    if (m) marks.push({ file: m[1].replace(/^[ab]\//, ''), at: i });
  });

  return marks.map((mark, k) => {
    const end = k + 1 < marks.length ? marks[k + 1].at : lines.length;
    let seg = lines.slice(mark.at + 1, end);

    // Chat UIs inject a language label and a "Copy" affordance above the block.
    // Both are only stripped when a fence follows, which is what makes them
    // chrome rather than content: without a fence there is nothing to label.
    const fenceFollows = (rest) => {
      let j = 0;
      while (j < rest.length && rest[j].trim() === '') j += 1;
      return j < rest.length && /^\s*```/.test(rest[j]);
    };
    while (
      seg.length
      && (COPY_AFFORDANCE.test(seg[0].trim()) || LANGUAGE_LABELS.has(seg[0].trim().toLowerCase()))
      && fenceFollows(seg.slice(1))
    ) {
      seg = seg.slice(1);
    }
    while (seg.length && seg[0].trim() === '') seg = seg.slice(1);

    const opens = seg.findIndex((l) => /^\s*```/.test(l));
    if (opens === 0) {
      // Drop the opening fence and everything from the matching close onward.
      // Close on the LAST bare fence, not the first: a Markdown file carries
      // its own inner blocks, and cutting at the first close would silently
      // truncate the file while still reporting a successful write.
      const rest = seg.slice(1);
      let closes = -1;
      for (let i = rest.length - 1; i >= 0; i--) {
        if (/^\s*```\s*$/.test(rest[i])) { closes = i; break; }
      }
      seg = closes === -1 ? rest : rest.slice(0, closes);
    }

    // Exactly one trailing newline. `replace(/\s+$/, '\n')` only fires when
    // there is trailing whitespace to replace, so a body ending in a normal
    // character was written with no final newline at all.
    const stripped = seg.join('\n').replace(/^\n+/, '').replace(/\s*$/, '');
    // A truly empty file is zero bytes, not one newline.
    const body = stripped === '' ? '' : `${stripped}\n`;
    return { file: mark.file, body };
  });
  // Empty blocks are kept. An empty file is a valid complete content, and
  // dropping it made a reply that asked to blank a file look like a reply that
  // never mentioned it — a partial success reported as a full one.
}

(() => {
  const files = parseReply(fs.readFileSync(REPLY, 'utf8'));
  if (!files.length) {
    throw new Error(
      `no "FICHERO: <ruta>" blocks found in ${REPLY} — the implementer did not follow the ` +
      'contract. Re-ask; do not hand-transcribe the reply.'
    );
  }
  // An empty file is a legitimate complete content, so blocks are no longer
  // dropped for being blank. But a reply made entirely of empty blocks used to
  // report "no FICHERO blocks found", which denies something that was there
  // and sends the operator to re-ask for a reply that had in fact arrived.
  if (files.every((f) => !f.body.trim())) {
    throw new Error(
      `every "FICHERO:" block in ${REPLY} is empty. The headers are there, so the implementer ` +
      'answered — but no content survived. Re-ask rather than writing empty files.'
    );
  }

  // path.resolve alone is a textual check: a symlink inside the repository that
  // points elsewhere passes it, and writeFileSync then follows the link out.
  //
  // Containment therefore has three parts, and two earlier attempts got it
  // wrong by shipping only one of them:
  //
  //  - The repository itself is canonicalised, and targets are resolved against
  //    the canonical path. Resolving against the argument instead means a
  //    workspace reached through a symlink (/workspace -> /real/repo) produces
  //    targets that can never start with the canonical prefix, and every
  //    legitimate write is refused.
  //  - Every existing ancestor directory is canonicalised, which catches a
  //    directory symlink pointing out of the tree.
  //  - The final component is checked on its own. The ancestor walk starts at
  //    the parent, so when the target *is* the symlink its parent looks
  //    perfectly legitimate and the write follows the link straight out.
  //
  // What this defends against is a hostile *reply*: a path designed to escape,
  // or a link already sitting in the tree. It is not proof against a second
  // process mutating the workspace while this one runs. O_NOFOLLOW covers the
  // final component, so a link planted there after validation fails — but an
  // intermediate directory swapped for a symlink in that window is still
  // followed, by mkdirSync and openSync alike. Closing it needs
  // descriptor-relative resolution (openat2, RESOLVE_BENEATH), which Node does
  // not expose. This is stated rather than glossed: an earlier version of this
  // comment called the three checks complete containment, which overstated
  // them, and an overstated guarantee is worse than a narrow one.
  const repoReal = fs.realpathSync(path.resolve(REPO));
  const inside = (p) => p === repoReal || p.startsWith(repoReal + path.sep);

  for (const { file } of files) {
    const target = path.resolve(repoReal, file);
    if (!inside(target)) {
      throw new Error(`refusing to write outside the repository: ${file}`);
    }
    let ancestor = path.dirname(target);
    while (!fs.existsSync(ancestor) && ancestor !== path.dirname(ancestor)) {
      ancestor = path.dirname(ancestor);
    }
    if (!inside(fs.realpathSync(ancestor))) {
      throw new Error(
        `refusing to write through a symlink that leaves the repository: ${file}`
      );
    }
    // lstat, not stat: the question is what the last component *is*, not what
    // it points at. A symlink is refused whatever its destination — one that
    // stays inside the tree is still the implementer redirecting a write.
    let self = null;
    try {
      self = fs.lstatSync(target);
    } catch {
      /* does not exist yet, which is the ordinary case for a new file */
    }
    if (self && self.isSymbolicLink()) {
      throw new Error(`refusing to write through a symlink: ${file}`);
    }
  }

  for (const { file, body } of files) {
    const target = path.resolve(repoReal, file);
    if (CHECK) {
      console.error(`would write ${file} (${Buffer.byteLength(body)} bytes)`);
      continue;
    }
    fs.mkdirSync(path.dirname(target), { recursive: true });
    // O_NOFOLLOW closes the gap between the check above and the write: the
    // kernel refuses to open a final component that is a symlink, so a link
    // planted after validation fails with ELOOP instead of being followed.
    // The lstat check is still worth keeping — it names the problem, while
    // this only reports an errno.
    let fd;
    try {
      fd = fs.openSync(
        target,
        fs.constants.O_WRONLY | fs.constants.O_CREAT | fs.constants.O_TRUNC | fs.constants.O_NOFOLLOW,
        0o666
      );
    } catch (e) {
      if (e.code === 'ELOOP') {
        throw new Error(`refusing to write through a symlink: ${file}`);
      }
      throw e;
    }
    try {
      fs.writeFileSync(fd, body);
    } finally {
      fs.closeSync(fd);
    }
    console.error(`wrote ${file} (${Buffer.byteLength(body)} bytes)`);
  }
})();
