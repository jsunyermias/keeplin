VEREDICTO: OBSERVACIONES
Hallazgos
OBS-1 — El test de collect.js pasa sin demostrar la garantía que declara
Fichero: scripts/tests/contract.test.js, líneas 181-195 (test 'collect refuses when the project contract is absent')
Qué garantía rompe: El test afirma verificar que collect.js rechaza la ausencia de AGENTS.md, pero el flujo de ejecución nunca alcanza esa validación. El script falla antes, en git fetch origin refs/pull/1/head:…, porque el repositorio temporal no tiene remoto origin ni existe el PR 1. La aserción assert.match(r.stderr, /AGENTS\.md|required project context|fatal/) se satisface con el fatal: de git, no con el mensaje de contexto faltante.
Escenario de fallo: Si alguien refactoriza o elimina el bloque REQUIRED/missing de collect.js (líneas 89-103), este test seguiría pasando en verde. La garantía de que una revisión no puede ejecutarse sin el contrato del proyecto queda sin cobertura real.
Corrección mínima: Crear un repositorio temporal con al menos un commit y un ref que simule refs/pull/1/head (por ejemplo, git update-ref refs/remotes/origin/pr/1 HEAD), y sin AGENTS.md. Así el fetch succeeds y la ejecución alcanza la comprobación de contexto. Alternativamente, separar la validación de contexto en una función exportable y testearla directamente.
OBS-2 — apply-files.js puede eliminar la primera línea de un fichero si el implementador omite las vallas
Fichero: scripts/apply-files.js, línea 49
js
1
while (seg.length && /^(Copy|Copiar|[a-z]{1,12})$/i.test(seg[0].trim())) seg = seg.slice(1);
Qué garantía rompe: Si la respuesta del implementador no incluye la valla de apertura (contrato violado) y el contenido del fichero comienza con una palabra única de ≤12 letras (import, const, tests, use…), el bucle la interpreta como chrome de la UI y la descarta. El fichero se escribe truncado sin error.
Escenario de fallo: Un implementador responde FICHERO: src/lib.rs seguido directamente de use std::io; sin valla. use (3 letras) se elimina; el fichero escrito empieza en std::io; y compila con error o, peor, compila con semántica alterada.
Mitigación existente: El contrato documentado exige vallas, y el filtro .filter((f) => f.body.trim()) descarta cuerpos vacíos. La probabilidad es baja en operación normal.
Corrección mínima: Restringir el stripping a las etiquetas conocidas de UI (Copy, Copiar, nombres de lenguaje de programación) en lugar de cualquier palabra [a-z]{1,12}, o exigir que la línea siguiente tras el stripping sea una valla; si no lo es, restaurar el segmento original.
OBS-3 — Consistencia de REVIEW_PHRASE entre build-prompt.js y gate.js depende de una variable de entorno no documentada como contrato
Ficheros: scripts/build-prompt.js línea 44; scripts/gate.js línea 16
Ambos leen process.env.REVIEW_PHRASE con el mismo default. Si un operador exporta la variable para uno de los dos procesos y no para el otro (por ejemplo, en un script de orquestación que invoca build-prompt.js con REVIEW_PHRASE=X y luego gate.js sin ella), el gate buscará una frase distinta de la que el prompt instruyó. El resultado es un falso negativo permanente (exit 1 en cada ciclo hasta el tope).
Corrección mínima: Documentar en SKILL.md que REVIEW_PHRASE debe exportarse para toda la sesión o no usarse; alternativamente, que gate.js acepte --phrase <string> explícito para eliminar la dependencia ambiental.
OBS-4 — SKILL.md dice "Qwen and GLM run in parallel" pero el ejemplo los ejecuta en secuencia
Fichero: SKILL.md, sección "Running a cycle", paso 4, y sección "Why it is built this way", párrafo 2.
La propiedad real es ceguera mutua (mismo prompt, sin ver la respuesta del otro), no paralelismo temporal. El ejemplo muestra dos invocaciones secuenciales de ask.js. Un lector que espere paralelismo real (por ejemplo, con & en shell) podría omitir la dependencia de que ambos terminen antes del paso 5.
Corrección mínima: Sustituir "run in parallel" por "run independently (neither sees the other's reply)" o añadir una nota de que la ejecución puede ser secuencial o paralela indistintamente.
OBS-5 — apply-patch.js no limpia tmpDir en el camino de error
Fichero: scripts/apply-patch.js, líneas 107-139
fs.rmSync(tmpDir, …) solo se ejecuta tras un git apply exitoso (línea 136). Si el patch no aplica (línea 121, throw new Error(…)), el directorio temporal con el fichero de patch permanece en /tmp. No es un problema de seguridad (el contenido es un diff ya presente en la respuesta del modelo), pero en un bucle de ciclos fallidos se acumulan directorios huérfanos.
Corrección mínima: Envolver el bloque principal en try/finally con la limpieza en finally, o registrar un handler process.on('exit', …).
Preguntas y supuestos
No puedo ejecutar los tests (node --test scripts/tests/contract.test.js) ni los scripts desde este entorno; la verificación es estática sobre el diff. La corrección de OBS-1 está confirmada por lectura del flujo, no por ejecución.
No tengo acceso a los drivers de navegador (qwen-web-chat, glm-web-chat, kimi-web-chat) ni a codex exec; las garantías sobre extracción de respuestas y sandbox read-only se aceptan por la documentación del SKILL.md y no se verifican independientemente.
El diff es el resultado integrado desde merge-base. No se muestra la eliminación de codex.js mencionada en el contexto del ciclo; asumo que fue añadido y eliminado dentro de la rama, con diff neto cero.
No puedo verificar la compatibilidad cross-repo (keeplin ↔ keeplin-srv) porque el cambio es exclusivamente en .claude/skills/ y no toca superficies compartidas.
Resumen
Las siete correcciones del ciclo 1 están correctamente implementadas: el implementador es fijo por PR (derivado del número, sin parámetro de ciclo); el gate exige la frase exactamente una vez y como última línea no vacía; collect.js transporta AGENTS.md y falla si falta; SKILL.md documenta los códigos de salida; apply-files.js solo retira las vallas exteriores; apply-patch.js usa mkdtempSync; y existen tests de contrato offline. No se han introducido regresiones sobre lo corregido.
Los cinco hallazgos son observaciones: un test que pasa por la razón equivocada (OBS-1), un edge-case de parsing con contrato violado (OBS-2), una dependencia ambiental no documentada (OBS-3), una imprecisión terminológica en la documentación (OBS-4) y una limpieza de temporales incompleta (OBS-5). Ninguno impide el funcionamiento correcto en el flujo documentado ni compromete la garantía de independencia de la revisión.
El contenido generado por IA puede no ser preciso.
