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

const argv = process.argv.slice(2);
const CHECK = argv.includes('--check');
const [REPO, REPLY] = argv.filter((a) => !a.startsWith('--'));

const HEADER = /^\s*(?:FICHERO|FILE|ARCHIVO)\s*:\s*(\S+)\s*$/i;

// The reply arrives as rendered text, so fences may or may not survive. Take
// everything between one header and the next as that file's body.
//
// Only the block's own outer fences are removed — the opening one just after
// the header and its matching close. Stripping every line that starts with a
// fence would quietly gut a Markdown file's inner code blocks, turning an
// apparently successful write into altered content, which is worse than a
// visible failure.
function parse(text) {
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
    while (seg.length && /^(Copy|Copiar|[a-z]{1,12})$/i.test(seg[0].trim())) seg = seg.slice(1);
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

    const body = seg.join('\n').replace(/^\n+/, '').replace(/\s+$/, '\n');
    return { file: mark.file, body };
  }).filter((f) => f.body.trim());
}

(() => {
  if (!REPO || !REPLY) {
    throw new Error('usage: node apply-files.js <repo-path> <reply-file> [--check]');
  }

  const files = parse(fs.readFileSync(REPLY, 'utf8'));
  if (!files.length) {
    throw new Error(
      `no "FICHERO: <ruta>" blocks found in ${REPLY} — the implementer did not follow the ` +
      'contract. Re-ask; do not hand-transcribe the reply.'
    );
  }

  for (const { file } of files) {
    // Keep a stray absolute or parent path from writing outside the repository.
    const target = path.resolve(REPO, file);
    if (!target.startsWith(path.resolve(REPO) + path.sep)) {
      throw new Error(`refusing to write outside the repository: ${file}`);
    }
  }

  for (const { file, body } of files) {
    const target = path.resolve(REPO, file);
    if (CHECK) {
      console.error(`would write ${file} (${Buffer.byteLength(body)} bytes)`);
      continue;
    }
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, body);
    console.error(`wrote ${file} (${Buffer.byteLength(body)} bytes)`);
  }
})();
