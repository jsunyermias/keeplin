model: GLM-5.2
url: https://chat.z.ai/c/934e47c7-5fa6-4f4b-9f49-5255758c8d05
--- reply ---
Show full message
Thought Process
VEREDICTO: OBSERVACIONES
Hallazgos
1. Comentario miente sobre la cobertura del test de flag mal escrito
contract.test.js, test a mistyped flag is refused, never treated as absent, ~línea 196.
El comentario dice: "Every command that mutates something is checked here, not just the one where it was noticed." El bucle solo prueba apply-files.js y apply-patch.js. collect.js (escribe en el directorio de salida), build-prompt.js (escribe prompt.txt) y ask.js (escribe la revisión y metadatos) también mutan el sistema de ficheros y no se prueban. Si el parser estricto reside en args.js y es compartido, el riesgo es bajo, pero el comentario afirma algo literalmente falso: es el tipo de defecto que este PR ha perseguido recurrentemente.
Corrección mínima: o bien extender el bucle para incluir collect.js, build-prompt.js y ask.js con entradas válidas para cada uno, o bien cambiar el comentario a "checked here for the two commands that mutate the repository; the rest inherit the same parser".
2. Comentario en gated afirma "Every gate test goes through --roles" cuando no es cierto
contract.test.js, función gated, ~líneas 17-20.
El comentario dice "Every gate test goes through --roles". El test gate treats an unparseable cycle as misuse, not as licence to iterate (~línea 335) no pasa --roles en ninguna de sus cinco invocaciones. Tampoco gate refuses to run at all without --roles (por diseño). El comentario es legible como "cada test que usa gated pasa por --roles", pero la palabra literal es "every gate test", y al menos dos no lo hacen.
Corrección mínima: cambiar a "Every test that uses this helper goes through --roles".
3. Tres tests de ask.js no pinan el motivo del rechazo
contract.test.js, tests ask refuses an empty reply (~línea 213), ask refuses a verdict that is not one of the three (~línea 265), ask requires the verdict on the first line (~línea 278).
Los tres verifican solo code === 1 y !fs.existsSync(out). No comprueban stderr ni el fichero .raw. Cualquier fallo en ask.js que produzca exit 1 y no escriba la revisión haría pasar el test, incluido un crash del proceso. El propio archivo advierte sobre este patrón en el comentario de run (~línea 8): una aserción sobre lo que una ejecución reporta pasa por el motivo equivocado si no se examina stderr.
En contraste, ask refuses a reply with no verdict line (~línea 186) sí comprueba /did not open with one of/, el .raw y la ausencia de .meta.json. Los tres tests débiles deberían hacer lo mismo con su mensaje específico.
Corrección mínima: añadir assert.match(r.stderr, /<motivo específico>/) a cada uno, y verificar el contenido de .raw cuando proceda.
4. gate refuses a blocking verdict even when the phrase is present no verifica que el gate reconoció BLOQUEANTE
contract.test.js, ~línea 62.
js
assert.strictEqual(r.code, 1);
assert.strictEqual(JSON.parse(r.stdout).cleared, false);
No comprueba JSON.parse(r.stdout).verdict. Si el gate no reconoce BLOQUEANTE y lo trata como verdict null (missing), el test pasaría igual: exit 1, cleared: false. El test certifica que el gate no cierra, pero no que distingue un veredicto bloqueante de una respuesta sin veredicto — y esos son caminos con mensajes distintos (/not the same as a clean review/ vs. algo específico de bloqueante).
Corrección mínima: assert.strictEqual(JSON.parse(r.stdout).verdict, 'BLOQUEANTE').
5. gate refuses when the phrase is repeated no pinta el motivo
contract.test.js, ~línea 75.
Solo comprueba code === 1. La frase aparece dos veces (en el cuerpo y como última línea). El gate podría rechazar por phrase_quoted_not_declared, por una regla de duplicación, o por cualquier otra razón, y el test pasaría. Comparar con gate treats a quoted phrase as a quotation, que sí pincha phrase_quoted_not_declared: true.
Corrección mínima: añadir la aserción sobre el campo específico que el gate reporta para el caso de frase repetida.
6. Sin test para build-prompt --prior
contract.test.js, cobertura ausente.
El issue describe build-prompt --prior como la ruta que adjunta revisiones previas por ruta, y menciona que el banner de 15 bytes del driver de GLM habría viajado al árbitro como si fuera la opinión de un revisor. No hay ningún test en este archivo que pase --prior a build-prompt.js. La corrección de ask.js (separar .raw de la revisión) no se verifica extremo a extremo a través de build-prompt.
No puedo verificar si --prior se prueba en otro archivo fuera de este diff.
Corrección mínima: un test que prepare un paquete con una revisión previa que contenga un banner sin veredicto en la ruta que --prior leería, pase build-prompt.js con --prior, y verifique que el banner no aparece en prompt.txt o que el script se niega a usar una revisión sin veredicto como prior.
Lo que no pude verificar
Implementación de gate.js, args.js, apply-files.js, apply-patch.js, ask.js, collect.js, build-prompt.js, roles.js: no están en el diff. No puedo confirmar que los mensajes de stderr que los tests aserten sean los que el código produce realmente, ni que el orden de comprobaciones (parser antes que --roles, verificación de identidad antes que verificación de PR) sea el que los tests asumen.
args.js hace validación semántica de --cycle: el test gate treats an unparseable cycle as misuse incluye [f, '--cycle', '0'] sin --roles y espera /bad argument/. Si args.js solo valida tipos (0 es numérico), el gate pasaría a comprobar --roles, reportaría --roles is required, y el match de /bad argument/ fallaría. Si el test pasa, args.js debe rechazar 0 semánticamente. No puedo confirmarlo.
ask.js comprueba el tamaño del prompt antes de hacer fork: el test ask refuses a prompt past the reviewer's measured input limit no usa stubCodex. Si ask.js hace fork antes de comprobar el tamaño y codex no está en PATH, el error sería distinto. No puedo confirmar el orden.
Que los 72 tests pasen: no puedo ejecutar la suite.
Si --prior se cubre en otro archivo de tests fuera de este diff.
Resumen
El archivo de tests es sólido en lo que cubre: las garantías críticas del gate (frase exacta, veredicto válido, identidad del revisor, tope de ciclos, PR correcto), las contenciones de apply-files (symlink en destino, en ruta intermedia, workspace vía symlink), y los modos de fallo de ask.js (banner sin veredicto, veredicto inválido, limpieza de revisión previa). Los hallazgos son de severidad baja: dos comentarios que afirman cobertura que no existe literalmente, tres tests de ask.js que no pinchan el motivo del rechazo (el patrón que el propio archivo denuncia), un test de gate que no verifica el reconocimiento de BLOQUEANTE, y la ausencia de cobertura para --prior. Ninguno es bloqueante para el contrato del test file, pero los comentarios falsos y los tests sin pinza de motivo son precisamente las clases de defecto que más han aparecido en este PR.
