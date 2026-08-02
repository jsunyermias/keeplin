// Assemble the prompt every reviewer receives for one cycle.
//
//   node build-prompt.js <collect-dir> <checklist.md> [meta.md]
//                        [--prior <review.md> ...] [--out <name>]
//
// Writes <collect-dir>/prompt.txt, or <collect-dir>/<name> with --out.
//
// With --prior it builds the adjudicator's prompt instead: the same change plus
// the reviews Qwen and GLM produced blind to each other. Those two run first
// and unaware of each other so their findings are two readings rather than one
// and an echo; the third arrives afterwards to say which reading survives.
//
// Either way the prompt carries the project's own review checklist, the
// objective, the diff, and the post-change text of the touched files —
// deliberately not the whole tree. Reviewers judge a change against its stated
// objective, and extra context mostly buys drift.
const fs = require('fs');
const path = require('path');

const argv = process.argv.slice(2);
const PRIOR = [];
let OUT_NAME = 'prompt.txt';
const positional = [];
for (let i = 0; i < argv.length; i++) {
  if (argv[i] === '--prior') {
    // Bounded to the two blind reviews. Consuming greedily swallowed the
    // positional meta.md when it followed the flag, and it was then attached
    // as a third "prior review" instead of the objective.
    while (PRIOR.length < 2 && argv[i + 1] && !argv[i + 1].startsWith('--')) {
      PRIOR.push(argv[++i]);
    }
  } else if (argv[i] === '--out') {
    OUT_NAME = argv[++i];
  } else {
    positional.push(argv[i]);
  }
}
const [DIR, CHECKLIST, META] = positional;

// Past this the diff starts crowding out the checklist in the model's
// attention, and a reviewer that skims is worse than one that declines.
const DIFF_WARN_BYTES = 200 * 1024;
// Whole-file context is a nicety; drop it rather than blow the budget.
const FILES_BUDGET_BYTES = 120 * 1024;
// The project contract comes first, but it must not crowd out the diff either.
const CONTEXT_BUDGET_BYTES = 60 * 1024;

// The gate demands this exact line as the last one; the adjudicator has to be
// told to emit it, or a clean review could never close the cycle.
const PHRASE = process.env.REVIEW_PHRASE || 'REVISION-COMPLETADA-SIN-BLOQUEANTES';

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

  // The checklist tells reviewers to read AGENTS.md and companions. They cannot
  // open the repository, so those travel in the prompt; what could not fit is
  // named, because a reviewer must know which rules it was unable to apply.
  const ctxDir = path.join(DIR, 'context');
  const ctxIncluded = [];
  const ctxOmitted = [];
  if (fs.existsSync(ctxDir)) {
    let ctxUsed = 0;
    const ctxParts = [];
    // AGENTS.md is required, the rest optional, and alphabetical order would
    // let a big optional file evict it under the budget. Required first.
    const REQUIRED_CTX = ['AGENTS.md'];
    const rank = (f) => (REQUIRED_CTX.includes(f.replace(/__/g, '/')) ? 0 : 1);
    const ctxFiles = fs.readdirSync(ctxDir)
      .sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
    for (const f of ctxFiles) {
      const body = read(path.join(ctxDir, f));
      const label = f.replace(/__/g, '/');
      // Required resources are exempt from the budget. Dropping the contract a
      // reviewer is told to apply does not save a review, it invalidates it.
      const required = rank(f) === 0;
      if (!required && ctxUsed + body.length > CONTEXT_BUDGET_BYTES) {
        ctxOmitted.push(label);
        continue;
      }
      ctxUsed += body.length;
      ctxIncluded.push(label);
      ctxParts.push(`### ${label}`, '', body.trim(), '');
    }
    if (ctxParts.length) {
      parts.push('## Contrato del proyecto', '', ...ctxParts, '---', '');
    }
    if (ctxOmitted.length) {
      parts.push(
        `Recursos del proyecto omitidos por tamaño: ${ctxOmitted.join(', ')}.`,
        'No des por verificada ninguna regla que dependa de ellos.',
        '', '---', ''
      );
    }
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

  if (PRIOR.length) {
    parts.push('', '---', '', '## Revisiones previas, hechas por separado', '');
    for (const p of PRIOR) {
      const name = path.basename(p).replace(/^review-|\.md$/g, '');
      parts.push(`### ${name}`, '', fs.readFileSync(p, 'utf8').trim(), '');
    }
  }

  parts.push(
    '',
    '---',
    '',
    'Responde en español. La primera línea debe ser exactamente uno de:',
    'VEREDICTO: BLOQUEANTE | VEREDICTO: OBSERVACIONES | VEREDICTO: SIN HALLAZGOS',
    ''
  );

  if (PRIOR.length) {
    // The adjudicator's job is to resolve, not to re-review from scratch and
    // not to defer. Prior findings are input, never authority — the same rule
    // AGENTS.md applies to the author's own explanation.
    parts.push(
      'Eres el tercer revisor y llegas después de los dos anteriores, que',
      'trabajaron por separado y sin verse. Tu trabajo es arbitrar:',
      '',
      '- Para cada hallazgo previo, di si lo confirmas, lo refutas o no puedes',
      '  decidirlo, y por qué, mirando el diff y no su argumentación.',
      '- Donde se contradigan, resuelve explícitamente cuál se sostiene.',
      '- Añade lo que ambos hayan pasado por alto.',
      '',
      'Que un hallazgo venga de otro revisor no lo hace cierto, y que ninguno lo',
      'mencione no lo hace inexistente. No repitas el diff.',
      '',
      'CIERRE OBLIGATORIO. Si y solo si tu veredicto es SIN HALLAZGOS, la ULTIMA',
      'linea de tu respuesta debe ser exactamente, sola y sin nada despues:',
      '',
      PHRASE,
      '',
      'No escribas esa linea en ningun otro sitio, ni la cites, ni la menciones al',
      'discutir hallazgos previos. Si tu veredicto no es SIN HALLAZGOS, omitela.'
    );
  } else {
    parts.push(
      'Después, un hallazgo por bloque: fichero y línea, qué garantía rompe, y qué',
      'lo demostraría. No repitas el diff. Si algo no puedes verificar desde lo que',
      'te he dado, dilo explícitamente en lugar de suponerlo.'
    );
  }

  const prompt = parts.join('\n');
  const out = path.join(DIR, OUT_NAME);
  fs.writeFileSync(out, prompt);

  console.error(`wrote ${out}: ${Buffer.byteLength(prompt)} bytes ` +
    `(diff ${Buffer.byteLength(diff)}, ${changed.length} files, ${skipped.length} omitted` +
    `, context ${ctxIncluded.length} in/${ctxOmitted.length} out` +
    `${PRIOR.length ? `, ${PRIOR.length} prior review(s)` : ''})`);
  if (Buffer.byteLength(diff) > DIFF_WARN_BYTES) {
    console.error('warning: diff is large; consider splitting the review by area');
  }
})();
