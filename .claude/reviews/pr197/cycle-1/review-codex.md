VEREDICTO: BLOQUEANTE

## Hallazgos

1. **Bloqueante — El gate no puede aplicar de forma fiable el contrato de doble señal.**  
   Archivos: `.claude/skills/multi-model-review/scripts/build-prompt.js:100-120`, `gate.js:42-59`, `ask.js:77`.

   `build-prompt.js` nunca ordena al adjudicador emitir `REVISION-COMPLETADA-SIN-BLOQUEANTES`, mientras `gate.js` la exige. Un veredicto limpio normal termina en exit 1 y, al llegar al límite, exit 2; lo confirmé ejecutando el script.

   Además, `text.includes(PHRASE)` acepta la frase citada en una discusión o en un prompt reflejado. Las revisiones previas adjuntas ya contienen literalmente la frase, y la salida real de Qwen demuestra que el navegador puede devolver el prompt ecoado. Por tanto, añadir únicamente la instrucción propuesta por GLM no basta: la señal puede faltar cuando corresponde o aparecer sin intención de cierre.

   Corrección mínima: emitir un marcador estructurado y exclusivo al final de la respuesta, validar una línea exacta y su posición —por ejemplo, última línea— y rechazar salidas con prompt ecoado o múltiples declaraciones. Añadir tests de ambos falsos positivos y falsos negativos.

2. **Bloqueante — El flujo operativo documentado omite por completo el gate contractual.**  
   Archivo: `.claude/skills/multi-model-review/SKILL.md:60-90`.

   El procedimiento termina con `grep` y ordena parar cuando todos dicen `SIN HALLAZGOS`. No invoca `gate.js`, no comprueba la frase y no aplica el máximo de ciclos. Un operador que siga el skill puede declarar terminada una revisión que el gate rechazaría.

   Corrección mínima: convertir `gate.js --cycle N --max-cycles M` en el paso obligatorio de cierre y documentar los significados de exit 0/1/2. La documentación debe usar exactamente el mismo criterio que el script.

3. **Bloqueante — Los revisores sin acceso al repositorio no reciben el contexto que el propio método les obliga a revisar.**  
   Archivo: `.claude/skills/multi-model-review/scripts/build-prompt.js:44-98`.

   El prompt incluye checklist, metadatos, diff y algunos archivos tocados, pero no `AGENTS.md`, ADRs, companions ni la tabla de verificación del issue. Qwen y GLM no pueden abrir esos recursos por sí mismos. El objetivo no demuestra así la revisión independiente exigida: varias garantías sólo pueden evaluarse ignorando instrucciones del checklist.

   Corrección mínima: recolectar e incluir `AGENTS.md`, la tabla del issue y companions/ADRs aplicables, con manifiesto de recursos incluidos y omitidos. Si falta un recurso obligatorio, la construcción del prompt debe fallar o marcar la revisión como no verificable.

4. **Bloqueante — La alternancia permite que un modelo arbitre posteriormente su propia implementación.**  
   Archivos: `.claude/skills/multi-model-review/scripts/roles.js:28-39`, `collect.js:42-46`.

   `roles.js` intercambia Kimi y Codex en cada ciclo, pero `collect.js` siempre entrega el diff acumulado desde el merge-base. En el ciclo 2, Kimi arbitra un diff que contiene lo que Kimi implementó en el ciclo 1. Esto contradice literalmente “quien implementa nunca revisa”.

   No acepto como corrección suficiente revisar sólo el delta del ciclo: eso dejaría de revisar el resultado integrado completo. La corrección mínima es mantener un único implementador durante toda la revisión del PR, o registrar procedencia por commit y elegir un adjudicador que no haya implementado ninguna parte del diff acumulado.

5. **Alta — `apply-files.js` corrompe contenido válido silenciosamente.**  
   Archivo: `.claude/skills/multi-model-review/scripts/apply-files.js:38-45`.

   El parser elimina todas las líneas que comienzan por triple backtick, no sólo las vallas exteriores. Un `README.md` u otro Markdown pierde las vallas de sus bloques internos. Esto contradice la afirmación de recuperación “byte for byte” y puede convertir una respuesta aparentemente aplicada con éxito en un archivo alterado.

   Corrección mínima: reconocer únicamente la valla de apertura inmediatamente posterior al encabezado y su valla de cierre correspondiente. Comparar luego el contenido aplicado con el bloque copiado, o usar un formato estructurado con longitud/hash.

6. **Alta — No hay verificación automatizada reproducible para las garantías críticas.**  
   Área: `.claude/skills/multi-model-review/`.

   No se aportan tests ni una tabla de verificación ejecutable para gate, rotación, parsing, traversal, prompts ecoados, truncamientos o códigos de salida. `node --check` pasa para los ocho scripts, y las cuatro pruebas manuales del gate produjeron los exits esperados por su implementación, pero eso no demuestra el contrato completo. Conforme al método solicitado, verificadores que no pueden ejecutarse ni citarse son hallazgos.

   Corrección mínima: tests con `node:test`, fixtures de respuestas reales y un verificador único que cubra al menos los casos anteriores.

7. **Media — El temporal de `apply-patch.js` admite sobrescritura mediante symlink.**  
   Archivo: `.claude/skills/multi-model-review/scripts/apply-patch.js:106-109`.

   `/tmp/impl-${pid}.patch` es predecible y `writeFileSync` sigue enlaces simbólicos. En un host compartido podría sobrescribir un archivo accesible al usuario del proceso.

   Corrección mínima: `mkdtempSync`, archivo creado con `O_CREAT|O_EXCL|O_NOFOLLOW` cuando esté disponible y limpieza en `finally`.

8. **Baja — `codex.js` es una vía muerta que contradice los prerrequisitos documentados.**  
   Archivos: `.claude/skills/multi-model-review/scripts/codex.js:1-76`, `ask.js:49-56`, `SKILL.md:53-58`.

   `ask.js` usa `codex exec` y autenticación ChatGPT; `codex.js` usa `OPENAI_API_KEY` y `/chat/completions`, precisamente la vía que el skill dice que no sirve en este entorno. Debe eliminarse o documentarse como utilidad alternativa no integrada.

   Refuto, no obstante, la afirmación de Qwen de que `codex.js` permite escribir en el árbol: una llamada de chat HTTP no obtiene por sí misma acceso al sistema de archivos. El problema es la contradicción operativa, no la falta de sandbox.

9. **Baja — Robustez insuficiente en entradas y archivos no textuales.**  
   Archivos: `.claude/skills/multi-model-review/scripts/build-prompt.js:27-37,55-71`, `collect.js:53-58`.

   Confirmo parcialmente los hallazgos de Qwen: `--prior` consume posiciones posteriores hasta la próxima opción y los binarios se decodifican como UTF-8. El primer caso no afecta al comando documentado, pero es una interfaz ambigua; el segundo puede desperdiciar el presupuesto y degradar la revisión.

   Corrección mínima: parser explícito de argumentos y detección/omisión declarada de archivos binarios.

## Arbitraje de las revisiones previas

- Qwen O-1 / GLM 3: **confirmados y elevados a bloqueante**.
- Qwen O-2: **confirmado**, ampliado a todo el contrato verificable.
- GLM 1: **confirmado**, pero su corrección propuesta es insuficiente por ecos y citas.
- GLM 4: **confirmado como bloqueante**.
- GLM 5: **confirmado**; revisar sólo el delta no preserva la revisión del resultado integrado.
- GLM 7: **confirmado**.
- Qwen O-4 / GLM 6: **confirmados**.
- Qwen O-3: **válido sólo para la utilidad no integrada `codex.js`**, severidad baja.
- Qwen O-5 y O-6: **confirmados con severidad baja**.
- Qwen O-7 / GLM 2: **confirmados en cuanto a código muerto y divergencia**, pero refuto que el script HTTP otorgue acceso de escritura.

## Preguntas y supuestos

El directorio disponible no es un checkout Git: es una recolección con el diff y archivos post-cambio. No contiene `AGENTS.md`, ADRs, el issue original ni su tabla de verificación. Tampoco pude ejecutar navegadores, sesiones autenticadas, `git fetch`, `git apply` sobre un repositorio real ni Codex CLI. Estas ausencias impiden aprobar el cambio y, además, reproducen el defecto de contexto señalado.

## Resumen

La separación de roles dentro de un solo ciclo está bien encaminada y los scripts son sintácticamente válidos, pero el cierre automatizado no coincide con el prompt ni con la documentación, puede confundirse mediante texto ecoado y la rotación termina permitiendo auto-revisión entre ciclos. El PR no satisface todavía su objetivo contractual.
