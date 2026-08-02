PR jsunyermias/keeplin#197, área `.claude/skills/multi-model-review/`, CICLO 4.

Objetivo: orquestar la revisión independiente que exige AGENTS.md a través de
varias familias de modelo, moviendo diffs y revisiones como ficheros para que
casi nada pase por el contexto del agente que orquesta.

Reglas: Qwen y GLM revisan siempre, en paralelo y sin verse. El tercer asiento
lo ocupa quien no implementó y arbitra con ambas revisiones delante. El
implementador es FIJO durante toda la vida del PR. El gate exige a la vez la
frase de cierre como última línea y un veredicto SIN HALLAZGOS.

HISTORIA. Ciclos 1, 2 y 3 devolvieron BLOQUEANTE. Todo lo señalado está
corregido y presente en este diff. Lo último, del ciclo 3:
 - apply-patch.js: PATCH_LINE ya reconoce las cabeceras extendidas de git
   (new file mode, deleted file mode, rename from/to, copy, modo, binarios),
   que antes truncaban el parche tras `diff --git` y hacían fallar toda
   creación, borrado o renombrado. Reproducido antes de corregir.
 - build-prompt.js: los recursos requeridos (AGENTS.md) quedan exentos del
   presupuesto de contexto; antes podían omitirse por tamaño.
 - collect.js: el fetch actualiza refs/remotes/origin/<base>, que antes se
   quedaba en FETCH_HEAD y podía dejar el merge-base obsoleto.
 - 20 tests offline, verificados por mutación (revertir una corrección hace
   fallar los tests que la cubren).
 - SKILL.md: el arbitraje por encima de ~70 KB corresponde a Codex; Kimi se
   truncó dos veces a ese tamaño y el gate lo reportó como revisor fallido.

Tu tarea: verificar si estas correcciones resuelven de verdad lo señalado y si
han introducido problemas nuevos. El diff es el resultado integrado completo
desde el merge-base.
