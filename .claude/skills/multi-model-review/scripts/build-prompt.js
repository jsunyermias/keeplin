// Assemble the prompt every reviewer receives for one cycle.
//
//   node build-prompt.js <collect-dir> <checklist.md> [meta.md]
//
// Writes <collect-dir>/prompt.txt. The prompt carries the project's own review
// checklist, the objective, the diff, and the post-change text of the touched
// files — deliberately not the whole tree. Reviewers judge a change against its
// stated objective, and extra context mostly buys drift.
const fs = require('fs');
const path = require('path');

const [DIR, CHECKLIST, META] = process.argv.slice(2);

// Past this the diff starts crowding out the checklist in the model's
// attention, and a reviewer that skims is worse than one that declines.
const DIFF_WARN_BYTES = 200 * 1024;
// Whole-file context is a nicety; drop it rather than blow the budget.
const FILES_BUDGET_BYTES = 120 * 1024;

const read = (p) => fs.readFileSync(p, 'utf8');

(() => {
  if (!DIR || !CHECKLIST) {
    throw new Error('usage: node build-prompt.js <collect-dir> <checklist.md> [meta.md]');
  }

  const diff = read(path.join(DIR, 'diff.patch'));
  const changed = read(path.join(DIR, 'changed-files.txt')).split('\n').filter(Boolean);

  let filesSection = '';
  let used = 0;
  const skipped = [];
  for (const f of changed) {
    const p = path.join(DIR, 'files', f);
    if (!fs.existsSync(p)) continue;                 // deleted by the PR
    const body = read(p);
    if (used + body.length > FILES_BUDGET_BYTES) {
      skipped.push(f);
      continue;
    }
    used += body.length;
    filesSection += `\n### ${f}\n\n\`\`\`\n${body}\n\`\`\`\n`;
  }

  const parts = [
    read(CHECKLIST).trim(),
    '',
    '---',
    '',
  ];

  if (META && fs.existsSync(META)) {
    parts.push('## Objetivo del cambio', '', read(META).trim(), '', '---', '');
  }

  parts.push('## Diff', '', '```diff', diff.trim(), '```', '');

  if (filesSection) {
    parts.push('---', '', '## Ficheros tocados, ya con el cambio aplicado', filesSection);
    if (skipped.length) {
      parts.push(
        '',
        `Omitidos por tamaño (juzga estos solo por el diff): ${skipped.join(', ')}`
      );
    }
  }

  parts.push(
    '',
    '---',
    '',
    'Responde en español. La primera línea debe ser exactamente uno de:',
    'VEREDICTO: BLOQUEANTE | VEREDICTO: OBSERVACIONES | VEREDICTO: SIN HALLAZGOS',
    '',
    'Después, un hallazgo por bloque: fichero y línea, qué garantía rompe, y qué',
    'lo demostraría. No repitas el diff. Si algo no puedes verificar desde lo que',
    'te he dado, dilo explícitamente en lugar de suponerlo.'
  );

  const prompt = parts.join('\n');
  const out = path.join(DIR, 'prompt.txt');
  fs.writeFileSync(out, prompt);

  console.error(`wrote ${out}: ${Buffer.byteLength(prompt)} bytes ` +
    `(diff ${Buffer.byteLength(diff)}, ${changed.length} files, ${skipped.length} omitted)`);
  if (Buffer.byteLength(diff) > DIFF_WARN_BYTES) {
    console.error('warning: diff is large; consider splitting the review by area');
  }
})();
