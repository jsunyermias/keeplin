model: GLM-5.2
note: hit the poll ceiling; reply may be truncated
url: https://chat.z.ai/c/9e7b7819-d43b-4f53-a3c3-166d22d4b3d6
--- reply ---
Show full message
Thought Process
Revisión independiente — PR jsunyermias/keeplin#197, CICLO 5
He leído AGENTS.md, el issue y el diff completo. Mi fuente es el diff, el código y los tests; la explicación del autor es contexto. No puedo ejecutar los tests ni CI; mis verificaciones son estáticas.
VEREDICTO: BLOQUEANTE
Hallazgos
B1 — apply-files.js: un symlink en el fichero destino itself bypassa la contención
Archivo: .claude/skills/multi-model-review/scripts/apply-files.js, líneas 83-95.
Qué garantía rompe: la protección que debería impedir escribir fuera del repositorio a través de symlinks —la corrección principal del ciclo 4— no cubre el caso en que el propio fichero destino es un symlink existente.
Escenario de fallo:
El bucle de contención parte de path.dirname(target), no de target:
javascript
let ancestor = path.dirname(target);
while (!fs.existsSync(ancestor) && ancestor !== path.dirname(ancestor)) {
  ancestor = path.dirname(ancestor);
}
const ancestorReal = fs.realpathSync(ancestor);
Si target ya existe como symlink (p. ej. config.json -> /home/user/.config/shared.json), path.dirname(target) es el directorio padre, que existe y es legítimo. El bucle no itera. ancestorReal es el realpath del padre, que está dentro del repo. La comprobación pasa. Entonces fs.writeFileSync(target, body) sigue el symlink y sobrescribe /home/user/.config/shared.json.
El test cubre solo symlinks directorio como ancestro (enlace/robado.js), no symlinks fichero como destino.
Corrección mínima: antes de escribir, si target existe, resolver su realpath y verificar contención:
javascript
if (fs.existsSync(target)) {
  const targetReal = fs.realpathSync(target);
  if (targetReal !== repoReal && !targetReal.startsWith(repoReal + path.sep)) {
    throw new Error(`refusing to write through a symlink that leaves the repository: ${file}`);
  }
}
Y añadir un test que cree un symlink fichero en el destino y verifique que se rechaza.
B2 — SKILL.md: el flujo documentado no usa --roles, derrotando la protección de rechazo del implementador
Archivo: .claude/skills/multi-model-review/SKILL.md, líneas 83-90 (pasos 5 y 7).
Qué garantía rompe: la corrección del ciclo 4 ("gate.js con --roles exige que la revisión venga del árbitro asignado y rechaza la del implementador") existe en el código pero no en el flujo documentado. Un operador que siga el SKILL.md no la activa.
Evidencia:
Paso 5 dice textualmente:
gate.js does not check who replied
Esto era cierto antes del fix y es falso ahora cuando se pasa --roles. La frase no fue actualizada.
Paso 7 invoca:
bash
node scripts/gate.js work/pr197/review-kimi.md --cycle 1 --max-cycles 5
Sin --roles. El gate no verifica quién produjo la revisión. Un revisor que fuera el implementador pasaría la barrera si su veredicto y la frase coinciden.
Los tests siempre pasan --roles (línea 173 del test), así que el comportamiento verificado y el documentado divergen.
Corrección mínima:
Actualizar el ejemplo del paso 7 a node scripts/gate.js work/pr197/review-kimi.md --cycle 1 --max-cycles 5 --roles work/pr197/roles-pr197.json (o la ruta donde roles.js escribió el fichero).
Corregir el comentario del paso 5: el gate sí verifica quién respondió cuando se pasa --roles, y por eso es obligatorio pasar el fichero de roles.
B3 — apply-files.js: falsos positivos cuando REPO es un symlink
Archivo: .claude/skills/multi-model-review/scripts/apply-files.js, líneas 83-84.
Qué garantía rompe: si la ruta del repositorio pasada como REPO es un symlink (común en workspaces), la comprobación textual falla para todo fichero y la herramienta se vuelve inutilizable.
Evidencia:
javascript
const repoReal = fs.realpathSync(path.resolve(REPO));   // /real/repo
const target = path.resolve(REPO, file);                 // /symlink/repo/file.js
if (target !== repoReal && !target.startsWith(repoReal + path.sep)) {
  throw new Error(`refusing to write outside the repository: ${file}`);
}
target usa REPO (con symlink), repoReal usa el realpath. La comparación falla porque /symlink/repo/file.js no empieza por /real/repo/.
Corrección mínima: usar repoReal como base para resolver el target: const target = path.resolve(repoReal, file);.
O1 — roles.js: la persistencia depende de cwd y se elude cambiando de directorio
Archivo: .claude/skills/multi-model-review/scripts/roles.js, línea 16.
Qué garantía rompe: la inmutabilidad de la asignación de roles —otra corrección del ciclo 4— se bypassa si el operador ejecuta roles.js desde directorios distintos en ciclos distintos. Cada directorio obtiene su propio roles-pr<N>.json, y una reasignación contradictoria no se detecta.
El SKILL.md no especifica --dir ni un directorio de trabajo canónico.
Corrección mínima: exigir --dir o defectar a una ruta derivada del workspace (p. ej. process.env.WORKSPACE || cwd), y documentar que debe ser consistente entre ciclos.
O2 — apply-patch.js: fuga de directorio temporal en caminos de error
Archivo: .claude/skills/multi-model-review/scripts/apply-patch.js, líneas 95-117.
Qué garantía rompe: mkdtempSync crea un directorio que solo se limpia en el camino de éxito (línea 119). Si git apply --check falla para ambos intentos (throw en línea 116) o si git apply --3way falla (throw implícito de execFileSync), el directorio queda en /tmp.
Corrección mínima: envolver en try/finally:
javascript
try {
  // ... lógica existente ...
} finally {
  fs.rmSync(tmpDir, { recursive: true, force: true });
}
O3 — gate.js --roles: no verifica que el fichero de roles corresponda al mismo PR
Archivo: .claude/skills/multi-model-review/scripts/gate.js, líneas 41-55.
Qué garantía rompe: el gate verifica meta.reviewer === roles.adjudicator pero no verifica roles.pr contra el PR bajo revisión. Si el operador pasa el fichero de roles de otro PR cuya asignación coincide, la comprobación pasa sin detectar el error.
El objeto de roles incluye pr: PR, pero el gate no lo consulta.
Corrección mínima: incluir el PR en el meta (ask.js ya podría recibirlo) y verificar coincidencia, o al menos advertir si roles.pr no está disponible para comparar.
Preguntas y supuestos
Drivers de browser no verificables: los ficheros qwen.js, glm.js, kimi.js viven en skills hermanas fuera de este diff. No puedo verificar la afirmación del issue de que "los drivers fallan si la respuesta extraída viene vacía". ask.js no añade su propia comprobación de salida vacía; confía en que el driver devuelva non-zero. Si el driver devuelve 0 con stdout vacío, el meta.json se escribe, el gate reporta "no VEREDICTO" y el ciclo no cierra —eso es seguro pero ineficiente.
REVIEW_PHRASE como env var: tanto build-prompt.js como gate.js leen process.env.REVIEW_PHRASE con el mismo defecto. Si se establece en un shell y no en el otro, el prompt y el gate usarían frases distintas y ninguna revisión limpiaría. No es un bug de código, pero convendría pasarla como argumento o fijarla en un fichero compartido.
No puedo ejecutar los 24 tests ni scripts/check-docs.sh. Mi verificación es estática: he leído cada test y trazado su aserción al código. Los tests cubren los comportamientos que dicen cubrir, salvo el caso de symlink-fichero (B1) y la divergencia documentación/código (B2).
Resumen
El diff integra un sistema de revisión multi-modelo que mueve diffs y revisiones como ficheros. Las correcciones del ciclo 4 están presentes en el código: el gate con --roles rechaza al implementador, roles.js persiste la asignación, ask.js registra quién respondió, y apply-files.js resuelve realpaths. Pero dos de esas correcciones tienen huecos explotables:
B1: la contención de symlinks en apply-files.js no verifica el destino si ya existe como symlink. Es el mismo tipo de bypass que motivó la corrección, en una rama no cubierta.
B2: el flujo documentado en SKILL.md no pasa --roles al gate y afirma que "gate.js does not check who replied", contradiciendo la corrección. Un operador que siga la documentación no activa la protección.
B3, O1, O2 y O3 son problemas menores de robustez y usabilidad que no impiden la fusión por sí solos, pero B1 y B2 sí.
