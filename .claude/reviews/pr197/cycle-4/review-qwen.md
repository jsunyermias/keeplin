VEREDICTO: OBSERVACIONES
Hallazgos
OBS-1 — gate.js: ninguna comprobación mecánica de que el fichero revisado no lo produjo el implementador
Fichero: scripts/gate.js, todo el módulo.
Qué garantía rompe: AGENTS.md exige que quien implementa no revise. roles.js calcula correctamente la asignación y los tests la verifican, pero gate.js acepta cualquier fichero de revisión sin comprobar qué familia lo generó. Si el orquestador —por error de guion, una variable de entorno equivocada o una reordenación de los pasos— pasa a gate.js la salida del implementador en lugar del adjudicador, el gate la evaluaría como si fuera una revisión independiente. El SKILL.md lo advierte en prosa («gate.js does not check who replied»), pero la advertencia no es un mecanismo.
Escenario de fallo: el orquestador ejecuta ask.js codex … para la implementación y, por un copy-paste del ejemplo del SKILL.md, pasa ese mismo fichero a gate.js. El gate evalúa el contenido, encuentra veredicto y frase, y devuelve exit 0. La revisión independiente no existió.
Corrección mínima: que ask.js escriba una cabecera JSON en la primera línea del fichero de salida ({"reviewer":"kimi","model":"K3","ts":…}) y que gate.js acepte un argumento --expected-reviewer <familia> que compare con esa cabecera. Si no coincide, exit 2. No hace falta criptografía; basta con que el error sea ruidoso en lugar de silencioso.
Severidad: observación. El procedimiento documentado y la estructura del pipeline hacen el error improbable, y el fallo es «se cuela una revisión no independiente», no corrupción de datos. Pero la regla de AGENTS.md es dura («binds every agent without exception») y aquí descansa en disciplina del orquestador, no en código.
OBS-2 — apply-files.js: el pelado de chrome UI puede comer líneas legítimas de código antes de la cerca
Fichero: scripts/apply-files.js, líneas 44-45 (el while con /^(Copy|Copiar|[a-z]{1,12})$/i).
Qué garantía rompe: la expresión elimina cualquier línea de hasta 12 caracteres alfabéticos entre la cabecera FICHERO: y la cerca de apertura. Si el modelo emite una nota breve sin cerca —function, import, const, description— esa línea desaparece silenciosamente y el fichero se escribe truncado, reportando éxito.
Escenario de fallo: el implementador responde con FICHERO: src/lib.rs seguido de use std::io; y luego el bloque en una cerca. use std::io; tiene 11 caracteres alfabéticos (sin contar :: y ;, que no coinciden con [a-z]), así que en este caso concreto no se pela. Pero implementation (14) no se pelaría mientras que description (11) sí. El riesgo es bajo porque el contrato exige cerca, y si no la hay el resultado ya es impredecible.
Corrección mínima: restringir el pelado a una lista blanca de etiquetas UI conocidas (Copy, Copiar, javascript, typescript, python, bash, text, code) en lugar de un patrón genérico [a-z]{1,12}.
Severidad: observación. El contrato del implementador exige cerca; sin ella, el resultado ya no es fiable. El pelado genérico es una red de seguridad para chrome UI, no para código.
OBS-3 — apply-patch.js: tres alternativas de PATCH_LINE no tienen test dedicado
Fichero: scripts/apply-patch.js, líneas 48-53; scripts/tests/contract.test.js.
Qué garantía rompe: la regex incluye copy from, copy to, old mode, new mode, dissimilarity index, Binary files y GIT binary patch. Los tests cubren new file mode, deleted file mode, rename from/to y similarity index. Las alternativas restantes son correctas por inspección —coinciden con la documentación de git-diff(1)—, pero si una futura refactorización las elimina, ningún test falla.
Escenario de fallo: un parche que cambia el modo de un fichero (old mode 100644 / new mode 100755) se trunca tras diff --git y git apply reporta «No valid patches», exactamente el defecto del ciclo 3 pero para un caso no cubierto.
Corrección mínima: añadir un test con un parche de cambio de modo (dos líneas, sin hunks) y otro con copy from/to. Son cuatro líneas de test cada uno.
Severidad: observación. La regex es correcta hoy; el riesgo es regresión futura.
OBS-4 — La restricción de tamaño del adjudicador (~70 KB → Codex) es documentación, no código
Fichero: SKILL.md, sección «The adjudicator needs the model with the most headroom»; scripts/build-prompt.js y scripts/roles.js (ausencia de comprobación).
Qué garantía rompe: el SKILL.md documenta que Kimi se trunca a ~95 KB y que la adjudicación por encima de ~70 KB debe ir a Codex. Ni build-prompt.js ni roles.js consultan el tamaño del prompt ensamblado para validar que el adjudicador elegido puede manejarlo. El orquestador debe comprobarlo a mano.
Escenario de fallo: el orquestador sigue el ejemplo del SKILL.md al pie de la letra (roles.js dice adjudicator: kimi), ensambla un prompt de 100 KB, lo envía a Kimi, y recibe una respuesta truncada. El gate la reporta como revisión fallida (exit 1), no como error de configuración. El ciclo se repite hasta el límite, perdiendo tiempo y tokens, antes de que nadie caiga en que el problema es el tamaño.
Mitigación existente: el gate falla cerrado (una respuesta truncada no tiene veredicto → exit 1, nunca exit 0). No se cuela una revisión incompleta. El coste es operativo, no de integridad.
Corrección mínima: que build-prompt.js, cuando se invoca con --prior, escriba el tamaño del prompt en collect.info y emita un warning a stderr si supera 70 KB y el adjudicador previsto (según roles.js) es una familia de navegador. O, más simple: que roles.js acepte un flag --prompt-bytes N y devuelva adjudicator: codex cuando N > 70 000 y el adjudicador natural sea kimi.
Severidad: observación. El fallo es ruidoso y seguro; el riesgo es un ciclo desperdiciado.
OBS-5 — collect.js: no valida que el directorio de salida esté fuera del repositorio
Fichero: scripts/collect.js, línea de validación de argumentos (líneas 22-24).
Qué garantía rompe: si OUT es un subdirectorio de REPO (p. ej., node collect.js /repo 197 /repo/work/pr197), los ficheros recogidos (diff, context, files) quedan dentro del árbol de trabajo. Un git status posterior los mostraría como untracked, y un git add -A accidental los incorporaría al siguiente commit.
Escenario de fallo: el orquestador usa una ruta relativa que resuelve dentro del repo. La siguiente operación de git incluye los artefactos de revisión. No es corrupción, pero ensucia el historial.
Corrección mínima: tres líneas tras la validación de argumentos:
javascript
1
2
3
if (path.resolve(OUT).startsWith(path.resolve(REPO) + path.sep)) {
  throw new Error('output directory must be outside the repository');
}
Severidad: observación. El SKILL.md muestra work/pr197 como ruta externa, y el operador controla la invocación.
Verificación de las correcciones del ciclo 3
Corrección
¿Resuelve lo señalado?
Evidencia
PATCH_LINE con cabeceras extendidas
Sí. La regex cubre todas las cabeceras extendidas de git-diff(1). Los tests de creación, borrado y renombrado lo ejercitan.
Tests apply-patch applies a patch that creates a file y …deletes and renames.
AGENTS.md exento del presupuesto
Sí. rank(f) === 0 omite la comprobación de presupuesto. El test con un AGENTS.md de 70 KB lo verifica.
Test build-prompt keeps the required contract whatever the budget.
collect.js actualiza refs/remotes/origin/<base>
Sí. El fetch incluye refs/heads/${base}:refs/remotes/origin/${base}. El merge-base se calcula contra la ref actualizada.
Correcto por inspección; el test de collect ejercita el fetch completo contra un bare repo.
20 tests offline
Sí. Contados 20 casos en contract.test.js. Cubren gate, roles, apply-files, apply-patch, build-prompt y collect.
Recuento directo del fichero.
SKILL.md: adjudicación >70 KB a Codex
Parcialmente. Documentado como procedimiento operativo; no impuesto en código. El gate falla cerrado si el reviewer se trunca.
OBS-4.
No he detectado regresiones introducidas por estas correcciones. La regex PATCH_LINE es estrictamente más permisiva que la anterior. La exención de presupuesto es aditiva. El refspec de fetch es aditivo.
Preguntas y supuestos
No puedo ejecutar los tests (node --test scripts/tests/contract.test.js) ni verificar la afirmación de mutación. La revisión se basa en inspección del diff y del contrato. La cuenta de 20 tests y su cobertura son verificables por lectura; la eficacia de mutación, no.
No tengo acceso a los drivers de navegador (qwen-web-chat, glm-web-chat, kimi-web-chat) ni a codex. La interacción de ask.js con ellos se verifica solo por la interfaz documentada (stdin/stdout, código de salida).
Supongo que los drivers eliminan el eco del prompt en el fichero de salida, como documenta el SKILL.md. Si un driver no lo hiciera, la frase aparecería en el eco y el gate rechazaría incluso una revisión limpia (fallo seguro, no falso positivo).
No puedo verificar la compatibilidad cross-repo (keeplin ↔ keeplin-srv) porque este PR no toca código Rust ni superficies compartidas. El cambio es enteramente tooling de revisión.
Resumen
El cambio es sólido. Las cinco correcciones del ciclo 3 resuelven los defectos señalados y no introducen regresiones visibles. Los 20 tests cubren las garantías críticas del pipeline —gate, roles, aplicación de parches y ficheros, presupuesto de contexto y contrato del proyecto— y fallan cerrado en los caminos de error que he podido trazar. Los cinco hallazgos son observaciones: ninguna rompe la integridad del gate ni permite que una revisión falsa se cuele como limpia. La más relevante (OBS-1) es que la identidad del revisor descansa en la disciplina del orquestador, no en una comprobación mecánica; las demás son robustez defensiva y cobertura de tests. No hay bloqueantes.
El contenido generado por IA puede no ser preciso.
