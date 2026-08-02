VEREDICTO: BLOQUEANTE
Hallazgos
BLOQUEANTE — apply-patch.js: PATCH_LINE trunca parches que crean, borran, renombran o cambian modo de ficheros
Fichero: .claude/skills/multi-model-review/scripts/apply-patch.js, línea 39 (definición de PATCH_LINE), línea 47 (bucle que la usa).
Qué garantía rompe: apply-patch.js debe aplicar un diff unificado producido por un implementador. La función trimToPatch acota el parche usando PATCH_LINE para distinguir contenido de parche del ruido del UI. La regex no reconoce las cabeceras extendidas de git diff: new file mode, deleted file mode, old mode, new mode, copy from, copy to, rename from, rename to, similarity index, dissimilarity index, Binary files ni GIT binary patch.
Escenario de fallo: Un diff que crea un fichero nuevo contiene new file mode 100644 en la segunda línea. trimToPatch acepta la línea diff --git (coincide con PATCH_LINE), pero la siguiente new file mode 100644 no coincide con ningún patrón ni es vacía, así que el bucle se rompe. El parche devuelto es solo diff --git a/new.js b/new.js. git apply --check falla, el script lanza "patch does not apply cleanly", y el operador re-pregunta al implementador — indefinidamente, porque el parche original era válido. Mismo efecto para borrados, renombrados, copias, cambios de modo y ficheros binarios. El test "applies a diff whose @@ header the model miscounted" solo modifica un fichero existente, por lo que no cubre este caso.
Corrección mínima: Añadir las cabeceras extendidas a PATCH_LINE:
js
const PATCH_LINE = /^(diff --git |index |new file mode |deleted file mode |old mode |new mode |copy from |copy to |rename from |rename to |similarity index |dissimilarity index |Binary files |GIT binary patch |--- |\+\+\+ |@@ |[ +\-\\])/;
Y añadir un test que cree y borre ficheros mediante apply-patch.js.
OBSERVACIÓN — build-prompt.js no tiene tests y AGENTS.md puede quedar fuera del prompt por presupuesto
Fichero: .claude/skills/multi-model-review/scripts/build-prompt.js, líneas 78–95 (presupuesto de contexto).
Qué garantía rompe: La garantía de que el prompt que reciben los revisores incluye el contrato del proyecto. AGENTS.md se prioriza en el ordenamiento (rank), pero sigue sujeta al presupuesto CONTEXT_BUDGET_BYTES = 60 * 1024. Si AGENTS.md excede 60 KB, se omite con una advertencia y el revisor no tiene base para verificar las reglas que se le piden. Además, build-prompt.js no tiene ningún test: la lógica de presupuestos, la inclusión/exclusión de contexto, el manejo de --prior y la generación de instrucciones de cierre no están verificadas.
Corrección mínima: Eximir AGENTS.md del presupuesto — incluirlo siempre sin importar el tamaño — y añadir al menos un test que verifique que AGENTS.md aparece en el prompt cuando existe en el directorio de contexto, y otro que verifique que --prior adjunta las revisiones anteriores.
OBSERVACIÓN — collect.js: git fetch con refspecs explícitos no actualiza la rama base
Fichero: .claude/skills/multi-model-review/scripts/collect.js, línea 46.
Qué garantía rompe: El diff debe calcularse contra la rama base actualizada. Al pasar refspecs explícitos a git fetch, el refspec configurado del remoto se ignora. El argumento base (p. ej. main) se resuelve como refs/heads/main y se almacena solo en FETCH_HEAD, sin actualizar refs/remotes/origin/main. La línea 48 usa origin/${base} para el merge-base, que puede apuntar a un commit obsoleto si la rama base ha avanzado desde el último fetch sin refspecs explícitos.
Corrección mínima:
js
git(['fetch', '--quiet', 'origin',
  `refs/pull/${PR}/head:${headRef}`,
  `refs/heads/${base}:refs/remotes/origin/${base}`,
]);
Preguntas y supuestos
Asumo que apply-patch.js se usa para diffs que pueden crear ficheros nuevos, no solo modificar existentes. Si su alcance previsto es solo modificación de ficheros existentes, el hallazgo bloqueante pasaría a observación.
No pude ejecutar los tests (node --test scripts/tests/contract.test.js) ni los checks de CI. El análisis se basa en la lectura del código y la traza manual de los caminos de ejecución.
No pude verificar los drivers de navegador (qwen.js, glm.js, kimi.js) ni codex exec porque no están en este diff.
Verifiqué que las correcciones del ciclo 3 resuelven lo señalado en ciclos anteriores: implementador fijo por PR (roles.js), gate con doble señal que distingue cita de declaración (gate.js), --prior acotado a dos (build-prompt.js), collect.js transporta AGENTS.md y falla si falta, apply-files.js respeta fences internos, apply-patch.js usa mkdtempSync, ask.js con timeout por driver. Los 17 tests son correctos en lo que verifican.
Resumen
Las correcciones del ciclo 3 son efectivas para los problemas señalados en ciclos anteriores y no introducen regresiones en las garantías ya verificadas. El hallazgo bloqueante es nuevo respecto a lo señalado: apply-patch.js trunca silenciosamente cualquier diff que contenga cabeceras extendidas de git, afectando creación de ficheros, borrados, renombrados y cambios de modo. La causa es la regex PATCH_LINE, que no las reconoce; el efecto es un bucle de re-preguntas al implementador porque el error se atribuye al parche en lugar de al truncamiento.
