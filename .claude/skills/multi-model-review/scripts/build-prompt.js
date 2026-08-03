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

// The documentation told operators to "drop the whole-file section" for a
// browser reviewer and pointed at a byte cap that does no such thing. When the
// change rewrites the files it touches, the diff and the post-change text are
// nearly the same bytes twice, and the pair overflows the chat.
const { parse } = require('./args');

const ARGS = parse(process.argv.slice(2), {
  name: 'build-prompt.js',
  positionals: ['collect-dir', 'checklist.md'],
  optionalPositionals: ['meta.md'],
  flags: { 'no-files': {} },
  options: { prior: { repeat: true }, out: {} },
  usage: 'build-prompt.js <collect-dir> <checklist.md> [meta.md] '
    + '[--prior <review.md>]... [--out <name>] [--no-files]',
});
const NO_FILES = ARGS.noFiles;
// Bounded to the two blind reviews, and now stated rather than achieved by a
// loop that stopped consuming: the old form swallowed the positional meta.md
// when it followed the flag and attached it as a third "prior review".
const PRIOR = ARGS.prior;
const OUT_NAME = ARGS.out || 'prompt.txt';
const [DIR, CHECKLIST, META] = ARGS.positional;

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
  if (PRIOR.length > 2) {
    throw new Error(
      `--prior takes at most the two blind reviews, got ${PRIOR.length}. The adjudicator's ` +
      'job is to resolve those two, not to read an unbounded pile.'
    );
  }
  for (const p of [CHECKLIST, ...PRIOR]) {
    if (!fs.existsSync(p)) throw new Error(`no such file: ${p}`);
  }
  // A mistyped meta.md used to be indistinguishable from "no objective was
  // asked for": the prompt was built without it, silently, contradicting the
  // contract that says the objective always travels.
  if (META && !fs.existsSync(META)) {
    throw new Error(
      `no such objective file: ${META}. It was asked for, so it must be readable — building ` +
      'the prompt without it would send reviewers a change with no stated purpose.'
    );
  }

  const diff = read(path.join(DIR, 'diff.patch'));
  const changed = read(path.join(DIR, 'changed-files.txt')).split('\n').filter(Boolean);

  // collect.js writes collect.info last, so its presence is the only evidence
  // that this package was assembled by the pipeline rather than put together by
  // hand. Without it there is no way to tell a file missing from files/ apart
  // from a file the pull request deleted — and that ambiguity is not academic:
  // a hand-made package with an empty files/ once produced a prompt carrying no
  // whole-file context at all, and the run reported "9 files" because it was
  // counting changed-files.txt. Three reviewers judged that change on the diff
  // alone and only one noticed.
  const infoPath = path.join(DIR, 'collect.info');
  if (!fs.existsSync(infoPath)) {
    throw new Error(
      `${DIR} has no collect.info, so it was not produced by collect.js and what is missing ` +
      'from files/ cannot be distinguished from what the pull request deleted. Rerun collect.js.'
    );
  }
  const info = read(infoPath);
  // A malformed collect.info used to disable its own check: no files_captured
  // line meant NaN, Number.isInteger(NaN) is false, and the comparison below
  // was skipped entirely. The presence of the file then proved nothing.
  const capturedLines = info.split('\n').filter((l) => /^files_captured:/.test(l));
  if (capturedLines.length !== 1 || !/^files_captured:\s*\d+$/.test(capturedLines[0])) {
    throw new Error(
      `${infoPath} has no single valid "files_captured: <n>" line, so it cannot vouch for the ` +
      'package. Rerun collect.js rather than trusting a manifest that does not parse.'
    );
  }
  const captured = Number(capturedLines[0].match(/^files_captured:\s*(\d+)$/)[1]);
  const area = (info.match(/^area:\s*(.+)$/m) || [])[1];
  const partial = area && area !== '(whole pull request)' ? area.trim() : null;

  let filesSection = '';
  let used = 0;
  let embedded = 0;
  const skipped = [];
  for (const f of changed) {
    const p = path.join(DIR, 'files', f);
    if (!fs.existsSync(p)) continue;                 // deleted by the PR
    const body = read(p);
    if (NO_FILES || used + body.length > FILES_BUDGET_BYTES) {
      skipped.push(f);
      continue;
    }
    used += body.length;
    embedded += 1;
    filesSection += `\n### ${f}\n\n\`\`\`\n${body}\n\`\`\`\n`;
  }

  if (embedded + skipped.length !== captured) {
    throw new Error(
      `collect.info records ${captured} captured file(s) but only ${embedded + skipped.length} ` +
      'could be read back. The package is incomplete; rerun collect.js rather than sending a ' +
      'prompt whose context does not match what it claims.'
    );
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

  // A reviewer that does not know its view is partial will report the absence
  // of things that are present just outside the slice, and the operator has no
  // way to tell that from a real finding.
  if (partial) {
    parts.push(
      `Esta revisión cubre solo \`${partial}\` del pull request, no el cambio entero.`,
      'Lo que quede fuera de esa ruta no está aquí y no es un hallazgo: no lo trates',
      'como ausente ni como incompleto. Otra vuelta cubre el resto.',
      '', '---', ''
    );
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
  } else if (changed.length) {
    // Silence here reads as "there was nothing to add". Say it outright, so a
    // reviewer knows which of its conclusions rest on the diff alone — and say
    // which reason applies, because they call for different suspicion. There
    // are three, and collapsing them told reviewers a change "only deletes
    // files" when in fact every file had been dropped for size.
    let why;
    if (NO_FILES) {
      why = 'No se adjunta el texto completo de los ficheros: se ha omitido a propósito para '
        + 'que el prompt quepa. El diff de este cambio reescribe casi por completo lo que toca, '
        + 'así que tienes el estado resultante a la vista.';
    } else if (skipped.length) {
      why = 'No se adjunta el texto completo de ningún fichero: todos superaban el presupuesto '
        + `de tamaño. Son: ${skipped.join(', ')}. Júzgalos solo por el diff.`;
    } else {
      why = 'No se adjunta el texto completo de ningún fichero: este cambio solo borra '
        + 'ficheros. Juzga únicamente por el diff.';
    }
    parts.push(
      '---', '', why,
      'Di explícitamente qué no has podido verificar por esta razón.',
      ''
    );
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

  // Report what went in, not what was asked for. The old line counted
  // changed-files.txt, so it said "9 files" about a prompt containing none, and
  // the operator read that as confirmation the context had travelled.
  // Deleted files are named rather than left as a silent shortfall. Counting
  // against changed.length made "3/5 embedded, 0 omitted" mean either two
  // deletions or two files lost from the package, with no way to tell which.
  const deleted = changed.length - embedded - skipped.length;
  console.error(`wrote ${out}: ${Buffer.byteLength(prompt)} bytes ` +
    `(diff ${Buffer.byteLength(diff)}, ${embedded}/${captured} files embedded, ` +
    `${skipped.length} omitted ${NO_FILES ? 'by --no-files' : 'by size'}` +
    `${deleted ? `, ${deleted} deleted by the pull request` : ''}` +
    `, context ${ctxIncluded.length} in/${ctxOmitted.length} out` +
    `${PRIOR.length ? `, ${PRIOR.length} prior review(s)` : ''})`);
  if (Buffer.byteLength(diff) > DIFF_WARN_BYTES) {
    console.error('warning: diff is large; consider splitting the review by area');
  }
})();
