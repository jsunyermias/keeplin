VEREDICTO: OBSERVACIONES
O1 — collect.js: la corrección del fetch no está cubierta por los tests, pese a la afirmación de verificación por mutación
Fichero: scripts/tests/contract.test.js, función repoWithPullRef (líneas ~250-265), tests 19-20.
Qué garantía rompe: El issue afirma "20 tests offline, verificados por mutación (revertir una corrección hace fallar los tests que la cubren)". La corrección de collect.js (fetch explícito de refs/heads/${base}:refs/remotes/origin/${base}) no está cubierta por mutación. Si se elimina esa línea del refspec, los tests 19 y 20 siguen pasando.
Escenario de fallo: repoWithPullRef ejecuta git fetch -q origin en su setup (línea ~263), lo que ya actualiza refs/remotes/origin/main al commit correcto antes de que collect.js corra. Cuando collect.js hace su fetch, la tracking ref ya está fresca, así que el refspec explícito de la base es redundante para el test. Un lector que confíe en la afirmación de mutación creerá que un regresión en este punto sería detectada; no lo sería.
Corrección mínima: El test debe simular una tracking ref obsoleta: tras el setup, empujar un nuevo commit a refs/heads/main en el origin sin hacer git fetch, de modo que refs/remotes/origin/main quede apuntando al commit anterior. Entonces collect.js debe actualizarla con su propio fetch, y el test debe verificar que el merge-base y el diff son correctos (no vacíos, no obsoletos). Además, los tests 19-20 no verifican diff.patch ni changed-files.txt en absoluto: solo comprueban el contexto y el código de salida.
O2 — apply-patch.js: fuga de directorio temporal en fallo
Fichero: scripts/apply-patch.js, líneas 95-101 (throw sin cleanup) y línea 130 (cleanup solo en éxito).
Qué garantía rompe: Limpieza de recursos en caminos de fallo.
Escenario de fallo: mkdtempSync crea un directorio en /tmp/impl-XXXX. Si git apply --check falla para ambos intentos, el script lanza un Error sin llamar fs.rmSync. El directorio y el parche temporal quedan en /tmp. En una ejecución continua de ciclos de revisión, los directorios se acumulan.
Corrección mínima: Envolver la lógica post-mkdtempSync en try { ... } finally { fs.rmSync(tmpDir, { recursive: true, force: true }); }.
O3 — apply-files.js: el stripper de etiquetas de UI puede eliminar la primera línea de un fichero sin fence
Fichero: scripts/apply-files.js, línea 38.
Qué garantía rompe: Integridad del contenido escrito — pérdida silenciosa de datos.
Escenario de fallo: La regex /^(Copy|Copiar|[a-z]{1,12})$/i elimina líneas que son una palabra corta antes de buscar el fence de apertura. Si el modelo no usa fence (infringe el contrato) y el fichero empieza con una palabra de 1-12 letras minúsculas (ej. import, hello, return, export), esa línea se elimina silenciosamente. El fichero se escribe con contenido alterado y se reporta como éxito.
Corrección mínima: Solo stripar etiquetas cuando se encuentra un fence de apertura a continuación. Si no hay fence, no stripar — el contenido es el contenido.
O4 — apply-patch.js: PATCH_LINE no reconoce líneas de parche binario (literal, delta)
Fichero: scripts/apply-patch.js, línea 36 (PATCH_LINE).
Qué garantía rompe: Truncamiento de parches binarios — aunque Binary files y GIT binary patch están en la regex, las líneas literal N y delta N que siguen no coincen ningún patrón, por lo que trimToPatch corta el parche ahí.
Escenario de fallo: Un modelo produce un diff con parche binario. trimToPatch detiene el escaneo en literal 100 (no coincide con [ +\\-\\\\] ni con ninguna palabra clave). El parche se trunca, git apply --check falla, y el error culpa al implementador de un parche que era correcto.
Corrección mínima: Añadir 'literal ', 'delta ' a la lista de prefijos en PATCH_LINE. La probabilidad es baja (los modelos de chat rara vez producen parches binarios), pero el modo de fallo es confuso.
O5 — build-prompt.js: comentario "exempt from the budget" contradice el código
Fichero: scripts/build-prompt.js, líneas 84-90.
Qué garantía rompe: Divergencia entre documentación interna y comportamiento.
Escenario de fallo: El comentario dice "Required resources are exempt from the budget", pero el código ejecuta ctxUsed += body.length para los recursos requeridos. Un AGENTS.md de 70 KB agota los 60 KB de presupuesto para ficheros opcionales (que se miden contra ctxUsed + body.length > CONTEXT_BUDGET_BYTES). El comportamiento es razonable (el tamaño total importa), pero el comentario induce a creer que un AGENTS.md grande no afecta a los opcionales.
Corrección mínima: Cambiar el comentario a "Required resources are never dropped, but they still count toward the total context budget for optional files."
Preguntas y supuestos
No pude ejecutar los tests. Verifiqué la lógica de los 20 tests por inspección del código. La afirmación de "20 tests" es precisa (conté 20). La afirmación de "verificados por mutación" es precisa para apply-patch.js (tests 15-16 cubren las cabeceras extendidas) y build-prompt.js (test 17 cubre la exención de AGENTS.md), pero no para collect.js (ver O1).
No pude verificar los drivers de browser (ask.js → qwen.js, glm.js, kimi.js) ni el comportamiento de Codex. Estos requieren servicios externos y sesiones activas. Solo verifiqué la lógica offline: parseo de argumentos, manejo de errores, extracción de veredicto.
Asumo que roles.js produce la asignación correcta para el PR 197. 197 % 2 = 1, ROTATING[1] = 'codex', implementador = codex, adjudicador = kimi. Esto coincide con el ejemplo del SKILL.md.
El SKILL.md documenta que la decisión de tamaño (>70 KB → Codex) es manual, no automatizada en código. El gate captura la truncación como revisor fallido, así que el peor caso es un ciclo desperdiciado, no una revisión falsa. Asumo que esto es aceptable por diseño.
Resumen
Las cinco correcciones del ciclo 3 están presentes en el diff y son correctas en su lógica. PATCH_LINE reconoce todas las cabeceras extendidas de git; build-prompt.js exime AGENTS.md del presupuesto; collect.js actualiza la tracking ref de la base; el gate requiere frase + veredicto simultáneos; el SKILL.md documenta el límite de tamaño para el adjudicador. No hay pérdida de datos, problemas de autenticación, ni carreras en el código offline.
Los hallazgos son de cobertura y robustez: el test del fix de collect.js no lo verifica (O1), hay una fuga de tmpdir en fallo (O2), un edge case de pérdida de contenido en apply-files.js sin fence (O3), un truncamiento teórico de parches binarios (O4) y un comentario inexacto (O5). Ninguno es bloqueante para la corrección del código, pero O1 significa que una de las afirmaciones de verificación del issue no se sostiene.
