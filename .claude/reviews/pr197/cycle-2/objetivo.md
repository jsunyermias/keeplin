PR jsunyermias/keeplin#197, área `.claude/skills/multi-model-review/`, CICLO 2.

Objetivo: orquestar la revisión independiente que exige AGENTS.md a través de
varias familias de modelo, moviendo diffs y revisiones como ficheros para que
casi nada pase por el contexto del agente que orquesta.

Reglas: Qwen y GLM revisan siempre, en paralelo y sin verse. El tercer asiento
lo ocupa quien no implementó, llega después con ambas revisiones y arbitra. El
implementador es FIJO durante toda la vida del PR; la alternancia es entre PRs.

El gate exige a la vez la frase de cierre y un veredicto SIN HALLAZGOS, con
tope de ciclos.

CONTEXTO DE ESTE CICLO: el ciclo 1 devolvió BLOQUEANTE con siete hallazgos.
Se han aplicado correcciones para todos ellos:
 1. roles.js ya no alterna por ciclo; el implementador es fijo por PR.
 2. build-prompt.js ordena la frase de cierre; gate.js la exige una sola vez en
    toda la respuesta y como última línea no vacía.
 3. collect.js transporta AGENTS.md y compañeros, y falla si faltan.
 4. SKILL.md termina en gate.js con los códigos de salida tabulados.
 5. apply-files.js solo quita las vallas exteriores del bloque.
 6. apply-patch.js usa mkdtempSync.
 7. Hay tests de contrato offline en scripts/tests/contract.test.js.
Además se eliminó codex.js (código muerto sin sandbox), se acotó --prior y se
omiten binarios.

Tu tarea es verificar si esas correcciones resuelven realmente lo señalado, y
si han introducido problemas nuevos. El diff es el resultado integrado completo
desde el merge-base, no solo el delta de este ciclo.
