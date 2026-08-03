model: GLM-5.2
url: https://chat.z.ai/c/1f32a3fc-df41-4ed9-9f81-ee4998dacb5c
--- reply ---
Show full message
Thought Process
VEREDICTO: BLOQUEANTE
Hallazgos
1. roles.js — referencia a positional no definido; el mensaje de error miente y crashea
Archivo: .claude/skills/multi-model-review/scripts/roles.js, línea 56 (aprox.)
js
const PR = Number(ARGS.positional[0]);
// ...
(() => {
  if (!Number.isInteger(PR) || PR < 1) {
    throw new Error(`pull request number must be an integer >= 1, got "${positional[0]}"`);
  }
positional no existe en ningún ámbito. En todo el archivo se usa ARGS.positional; la referencia desnuda aquí es un typo. Cuando se pasa un número de PR no numérico (p. ej. node roles.js abc --dir pkg), la validación se dispara correctamente, pero al construir el mensaje se accede a una variable indefinida y se lanza ReferenceError: positional is not defined en lugar del error legible. El operador ve un stack trace incomprensible en vez de got "abc".
La pregunta del issue fue explícita: «¿Miente algún contador, mensaje o comentario sobre lo que el código hace de verdad? Ese es el defecto que más veces ha aparecido aquí». Este mensaje afirma mostrar el valor rechazado y no lo hace: miente, y es el tipo de defecto que más se ha repetido en este PR.
Corrección mínima: cambiar positional[0] por ARGS.positional[0].
2. ask.js acepta veredictos con espaciado inicial que gate.js no puede leer
Archivos: .claude/skills/multi-model-review/scripts/ask.js (línea ~140) y .claude/skills/multi-model-review/scripts/gate.js (línea ~130)
ask.js usa ^\s*VEREDICTO: —permite espaciado inicial— para decidir si la respuesta es una revisión válida. gate.js usa ^VEREDICTO: —sin \s*— para extraer el veredicto del fichero. Si un modelo produce VEREDICTO: SIN HALLAZGOS (con espacios al inicio, lo cual ocurre), ask.js lo acepta, escribe el fichero de revisión y el .meta.json, y luego gate.js no encuentra veredicto y reporta «no VEREDICTO line — the reviewer failed». El operador ve un fichero que claramente contiene VEREDICTO: y un gate que dice que no, sin forma de reconciliarlo sin inspeccionar el código.
No es un cierre falso (el gate rechaza, que es seguro), pero desperdicia un ciclo de revisión —y con un tope de 5, gastar uno por un desajuste de regex es costoso— y el mensaje del gate miente sobre lo que tiene delante.
Corrección mínima: alinear los regex. La opción más segura es hacer gate.js igual de estricto que ask.js: cambiar ^VEREDICTO: a ^\s*VEREDICTO: en gate.js, o bien hacer ask.js igual de estricto que gate.js rechazando espaciado inicial.
3. ask.js — .raw no se limpia en la limpieza de stale
Archivo: .claude/skills/multi-model-review/scripts/ask.js, línea ~40
js
for (const stale of [OUT, `${OUT}.meta.json`, `${OUT}.err`]) {
    if (fs.existsSync(stale)) fs.rmSync(stale);
}
.raw no está en la lista. Si una ejecución previa tuvo éxito (y dejó .raw), y la nueva ejecución falla antes de llegar al spawnSync —por ejemplo, en la comprobación de tamaño de entrada—, el .raw anterior permanece. El operador que lo inspeccione para diagnosticar el fallo verá la salida de la ejecución anterior, no la actual. No afecta al gate (que no lee .raw), pero miente sobre el origen del contenido.
Corrección mínima: añadir ${OUT}.raw al array de stale.
4. gate.js — --cycle 0 se acepta sin validación
Archivo: .claude/skills/multi-model-review/scripts/gate.js, línea ~50
js
const CYCLE = ARGS && ARGS.cycle !== undefined ? ARGS.cycle : 1;
const MAX_CYCLES = ARGS && ARGS.maxCycles !== undefined ? ARGS.maxCycles : 5;
No hay comprobación de que CYCLE >= 1. --cycle 0 pasaría el check de cap (0 > 5 es falso, 0 >= 5 es falso) y se procesaría como un ciclo válido. El JSON de salida diría "cycle": 0, y si la revisión es limpia, el gate cerraría en ciclo 0 —un ciclo que no debería existir. No es peligroso (el veredicto sigue siendo válido), pero el contador miente sobre el estado del ciclo.
Corrección mínima: validar CYCLE >= 1 junto con el check de cap, o en args.js con un mínimo además de integer: true.
5. build-prompt.js — el reporte no distingue ficheros borrados de ficheros ausentes
Archivo: .claude/skills/multi-model-review/scripts/build-prompt.js, línea ~280
js
console.error(`wrote ${out}: ... ${embedded}/${changed.length} files embedded, ` +
    `${skipped.length} omitted ...`);
changed.length incluye ficheros borrados por el PR, pero embedded y skipped no los cuentan (se saltan con continue). Si un PR modifica 3 ficheros y borra 2, el reporte dice 3/5 files embedded, 0 omitted. El operador ve 3/5 con 0 omitted y no tiene forma de saber si los 2 que faltan son borrados legítimos o ficheros que se perdieron del paquete. La comprobación de collect.info ya verificó que el paquete está completo, pero el reporte no lo refleja.
Corrección mínima: reportar changed.length - embedded - skipped.length como deleted, o usar captured en vez de changed.length en el denominador.
6. roles.js — la nota del assignment sobre-dice la persistencia
Archivo: .claude/skills/multi-model-review/scripts/roles.js, línea ~100
js
note: 'assignment is recorded on first use and holds for every cycle of this pull request',
La persistencia es por directorio (--dir), no por PR. Si el operador usa un directorio distinto para un nuevo ciclo, se crea una asignación nueva e independiente. La nota dice «holds for every cycle of this pull request» sin matizar que es «in this directory». Un operador que cambie de directorio esperaría que la asignación persistiera y se encontraría con una nueva.
Corrección mínima: cambiar a «holds for every cycle of this pull request in this directory» o similar.
Preguntas y supuestos
args.js no está en el diff. Los cuatro archivos dependen de él para parsing, validación de enteros y conversión dash-to-camelCase. No puedo verificar que integer: true rechace NaN, que repeat: true funcione como se espera, o que el parser rechace flags duplicados y sobrantes. Cualquier bug en args.js afectaría a los cuatro archivos.
collect.js no está en el diff. No puedo verificar qué escribe exactamente en collect.info (formato de files_captured:, presencia de area:) ni en changed-files.txt. Las validaciones de build-prompt.js dependen de que collect.js produzca el formato esperado.
No tengo los tests. No puedo verificar que los 72 tests cubren lo que sus nombres prometen ni que pasan por el motivo correcto. El issue menciona tres casos previos donde los tests pasaban por el motivo equivocado; sin verlos, no puedo descartar un cuarto.
No puedo ejecutar los scripts. Mis hallazgos son del análisis estático del diff. No verifiqué el comportamiento real con node.
Resumen
El diseño del gate es sólido: los dos señales requeridos (veredicto + frase), la verificación de identidad y PR, y la escritura diferida en ask.js cierran las rutas de cierre falso que el issue describía. El hallazgo bloqueante es un typo que produce un ReferenceError donde debería haber un mensaje legible —exactamente el tipo de defecto que el issue pide buscar—. Las observaciones restantes son desajustes de regex entre ask.js y gate.js, limpieza incompleta de stale, validación de ciclo, y reportes que no distinguen borrados de ausentes. Ninguno permite un cierre falso, pero varios desperdician ciclos o confunden al operador.
