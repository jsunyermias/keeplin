PR jsunyermias/keeplin#197, área `.claude/skills/multi-model-review/`, CICLO 3.

Objetivo: orquestar la revisión independiente que exige AGENTS.md a través de
varias familias de modelo, moviendo diffs y revisiones como ficheros para que
casi nada pase por el contexto del agente que orquesta.

Reglas: Qwen y GLM revisan siempre, en paralelo y sin verse. El tercer asiento
lo ocupa quien no implementó, llega después con ambas revisiones y arbitra. El
implementador es FIJO durante toda la vida del PR; la alternancia es entre PRs.
El gate exige a la vez la frase de cierre como última línea y un veredicto
SIN HALLAZGOS, con tope de ciclos.

HISTORIA. El ciclo 1 devolvió BLOQUEANTE (7 hallazgos) y el ciclo 2 también
(4 de GLM, más 3 del árbitro cuya respuesta se truncó). Correcciones ya
aplicadas y presentes en este diff:
 - roles.js: implementador fijo por PR, nunca por ciclo.
 - gate.js: exige la frase una sola vez y como última línea no vacía; distingue
   cita de declaración; un veredicto ausente es revisor fallido, no limpio.
 - build-prompt.js: ordena la frase de cierre al árbitro; recursos requeridos
   antes que los opcionales en el presupuesto; --prior acotado a dos.
 - collect.js: transporta AGENTS.md y falla si falta; omite binarios.
 - SKILL.md: el ejemplo usa el árbitro que devuelve roles.js, no un nombre
   copiado; termina en gate.js con los códigos de salida tabulados.
 - apply-files.js: solo quita las vallas exteriores del bloque.
 - apply-patch.js: mkdtempSync; se eliminó repair(), que arreglaba un fallo
   que git no tiene (medido) mientras el fallo real no lo arregla ningún flag.
 - ask.js: timeout por driver; un timeout se reporta como no-revisión.
 - Se eliminó codex.js (código muerto, sin sandbox).
 - scripts/tests/contract.test.js: 17 tests offline, verificados por mutación.

Tu tarea: verificar si estas correcciones resuelven de verdad lo señalado y si
han introducido problemas nuevos. El diff es el resultado integrado completo
desde el merge-base, no el delta del ciclo.
