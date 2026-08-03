model: GLM-5.2
note: hit the poll ceiling; reply may be truncated
url: https://chat.z.ai/c/1a7bd7f0-e016-43eb-814d-1ea9c32b1700
--- reply ---
Show full message
Thought Process
VEREDICTO: OBSERVACIONES
Hallazgos
1. SKILL.md miente sobre lo que los tests verifican respecto a apply-patch.js y las líneas de contexto perdidas
Fichero: .claude/skills/multi-model-review/SKILL.md, sección Verification.
La sección dice textualmente: "that apply-patch.js recovers a diff whose blank context lines were rendered away and refuses prose". El test que cubre líneas de contexto perdidas —apply-patch refuses, and writes nothing, when a context line was lost— verifica que el script rechaza ese diff y no escribe nada, no que lo recupera. El test que sí recupera (applies a diff whose @@ header the model miscounted) opera sobre conteos erróneos en el header @@, un defecto distinto. La afirmación de la documentación invierte la garantía: promete recuperación donde el código ofrece rechazo protector.
Corrección mínima: cambiar "recovers a diff whose blank context lines were rendered away" por "refuses a diff whose blank context lines were rendered away".
2. Ningún test verifica que el gate rechaza SIN HALLAZGOS cuando la frase mágica está ausente
Fichero: .claude/skills/multi-model-review/scripts/tests/contract.test.js, tests del gate (~líneas 39-73 del nuevo fichero).
La matriz de tests del gate cubre:
SIN HALLAZGOS + frase como última línea → cleared (exit 0).
BLOQUEANTE + frase → rechazado (exit 1).
SIN HALLAZGOS + frase en el cuerpo (no al final) → rechazado (exit 1).
SIN HALLAZGOS + frase repetida → rechazado (exit 1).
Sin veredicto, sin frase → rechazado (exit 1).
Falta: SIN HALLAZGOS + frase ausente → debería ser rechazado (exit 1). Si el gate tuviera un bug donde acepta un veredicto limpio sin la frase, ningún test lo detectaría. La frase existe precisamente como confirmación deliberada que no puede ser accidental; su ausencia es el caso de cierre accidental más probable.
Corrección mínima: añadir un test con VEREDICTO: SIN HALLAZGOS\n\nTodo correcto.\n (sin PHRASE) y verificar exit 1.
3. El test "gate clears only when both signals agree" ejecuta gate.js sin --roles y demuestra que el gate cierra sin comprobar quién respondió
Fichero: contract.test.js, test ~línea 39; SKILL.md, paso 7.
El test ejecuta gate.js [fichero] sin --roles y verifica exit 0 (cleared). SKILL.md dice: "--roles is not optional either. Without it the gate never checks who replied, so a clean verdict from the implementer closes the cycle." El código no exige --roles: lo trata como opcional y cierra sin él. Esto es exactamente la vulnerabilidad que SKILL.md describe. Un operador que omita --roles obtiene un cierre sin la verificación de independencia — la protección para la que se escribió esta pieza.
Corrección mínima: o bien gate.js exige --roles (exit 2 si falta), o el test documenta explícitamente que es una prueba del núcleo lógico sin la verificación de independencia.
4. El test "apply-files refuses a symlink target even when it stays inside the repository" no comprueba r.stderr ni que el symlink siga existiendo
Fichero: contract.test.js, test ~línea 200.
A diferencia de los otros tests de symlinks —que verifican r.stderr con assert.match— este solo comprueba r.code === 1 y que real.js no cambió. Si apply-files.js fallara por una razón distinta (p. ej. un error de parseo), el test pasaría. Además, si el script eliminara el symlink y escribiera un fichero regular en su lugar, real.js seguiría intacto y el test también pasaría.
Corrección mínima: añadir assert.match(r.stderr, /symlink/) y verificar fs.lstatSync(path.join(repo, 'alias.js')).isSymbolicLink().
5. El test "ask records the reviewer and pull request of a real review" no verifica que el fichero de revisión se escribió
Fichero: contract.test.js, test ~línea 160.
El test comprueba r.code === 0, r.stdout contiene el veredicto, y .meta.json tiene reviewer y pr. Pero no verifica fs.existsSync(out) ni el contenido del fichero de revisión. Si ask.js escribiera .meta.json pero no el fichero de revisión, el test pasaría. El caso negativo ("ask refuses a reply with no verdict line") sí verifica !fs.existsSync(out), lo que hace la asimetría notable.
Corrección mínima: añadir assert.ok(fs.existsSync(out)) y assert.match(fs.readFileSync(out, 'utf8'), /VEREDICTO: OBSERVACIONES/).
6. Ningún test verifica que el gate rechaza una revisión de un revisor ciego (Qwen/GLM) que no es el árbitro
Fichero: contract.test.js, gap en tests del gate con --roles.
Los tests verifican: reviewer === implementer → rechazado (exit 2); reviewer === adjudicator + pr correcto → cleared (exit 0). Pero ningún test cubre reviewer: 'qwen' (un revisor ciego que no es implementer ni árbitro) con el PR correcto. Si el gate solo comprueba reviewer !== implementer, una revisión de Qwen cerraría el ciclo. No puedo confirmarlo sin ver gate.js.
Corrección mínima: añadir un test con reviewer: 'qwen' y pr: 197 en el meta, verificar exit 2 con un mensaje que indique que el revisor no es el árbitro.
7. Ningún test verifica la persistencia de un implementador fuera de la rotación
Fichero: contract.test.js, test "roles can record an implementer from outside the rotation" (~línea 260).
El test verifica que roles.js 197 claude --dir <dir> devuelve implementer: 'claude'. Pero no verifica que una segunda llamada roles.js 197 --dir <dir> (sin el argumento explícito claude) devuelva el mismo implementador. El test "the implementer cannot be reassigned once the review has started" solo cubre el caso por defecto (sin nombre explícito). Si roles.js no persiste el implementador fuera de rotación, una llamada posterior sin el nombre produciría un implementer distinto, y el gate con --roles correría con un archivo de roles inconsistente.
Corrección mínima: después de roles.js 197 claude --dir <dir>, ejecutar roles.js 197 --dir <dir> y verificar implementer === 'claude'.
8. SKILL.md paso 5 atribuye el arbitraje a Kimi para PR 197, pero la descripción del PR dice que el árbitro es Codex
Fichero: SKILL.md, paso 5.
El paso 5 dice: "roles.js said adjudicator: kimi for this pull request" y envía a kimi. Pero la descripción del PR dice: "el árbitro es Codex". El SKILL.md usa PR 197 como ejemplo pero muestra el camino por defecto (sin claude), que produce implementer: codex, adjudicator: kimi — distinto a la asignación real (implementer: claude, adjudicator: codex). El SKILL.md advierte "Use what it returned, never a name copied from an example", pero el ejemplo mismo es inconsistente con el PR que documenta.
Corrección mínima: usar un número de PR genérico en el ejemplo o alinear el comentario con la asignación real de PR 197.
Lo que no pude verificar
No pude ejecutar los tests — no tengo acceso al repositorio ni a un entorno Node.js.
No pude ver gate.js, ask.js, roles.js, apply-files.js, apply-patch.js, collect.js ni build-prompt.js — esta revisión se limita a contract.test.js y SKILL.md. Los hallazgos 2, 3 y 6 son gaps de cobertura que podría confirmar o descartar viendo gate.js.
No pude verificar las afirmaciones sobre drivers de navegador en SKILL.md (waitForReply, Thinking... indicator, clipboard) — los tests son offline.
No pude verificar los límites de tamaño empíricos de GLM, Kimi y Codex — solo el test de Qwen a 131072 caracteres.
La descripción del PR dice 49 tests; cuento 51 llamadas a test() en el fichero. Discrepancia factual menor.
Resumen
Los tests son sólidos en su núcleo — cubren los defectos que el PR corrige y cada test negativo se acompaña de su positivo. Los hallazgos son gaps de cobertura y una inexactitud documental, no bugs confirmados en código. El más importante es el hallazgo 1: SKILL.md afirma que los tests verifican recuperación de líneas de contexto perdidas cuando el test verifica rechazo — exactamente el tipo de afirmación invertida que el PR dice cazar. El hallazgo 2 (clean verdict sin frase, sin test) y el hallazgo 6 (revisor no-árbitro, sin test) dejan sin verificar dos caminos por los que el gate podría cerrar un ciclo que no debería cerrar.
