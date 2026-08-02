PR jsunyermias/keeplin#197, área `.claude/skills/multi-model-review/`.

Objetivo: orquestar la revisión independiente que exige AGENTS.md a través de
varias familias de modelo, moviendo diffs y revisiones como ficheros para que
casi nada pase por el contexto del agente que orquesta.

Reglas del ciclo: Qwen y GLM revisan siempre, en paralelo y sin verse entre sí.
El tercer asiento alterna con el implementador (Kimi o Codex) y llega después
con ambas revisiones delante, para arbitrar. Quien implementa nunca revisa.

El gate exige a la vez la frase REVISION-COMPLETADA-SIN-BLOQUEANTES y un
veredicto SIN HALLAZGOS, con tope de ciclos.

No cambia Rust ni comportamiento en ejecución: es utillaje de agente.
