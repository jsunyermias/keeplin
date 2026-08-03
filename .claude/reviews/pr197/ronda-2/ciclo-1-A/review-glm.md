model: GLM-5.2
url: https://chat.z.ai/c/42b12806-c536-457a-827b-bee56363e6f9
--- reply ---
Show full message
Thought Process
VEREDICTO: BLOQUEANTE
1. apply-patch.js:17 — La regex de fences trunca silenciosamente diffs que contienen ``` y el resultado puede aplicar como parche corrupto
Fichero y línea: .claude/skills/multi-model-review/scripts/apply-patch.js, línea 17.
Qué garantía rompe: Que el parche que se aplica es el que el implementador produjo. La regex ```(?:diff|patch)?\n([\s\S]*?)``` no ancla el cierre al inicio de línea. Tres backticks en cualquier punto del contenido del diff —un +```` o -```` en un diff que añade o elimina un bloque de código en un Markdown— se matchean como cierre del fence exterior. La captura se corta ahí.
Escenario de fallo: Un diff que añade un bloque de código a un archivo Markdown:
text
diff --git a/file.md b/file.md
--- a/file.md
+++ b/file.md
@@ -1,3 +1,5 @@
 # Title
+```
+code
+```
 Some text.
La regex captura hasta los tres backticks de +```` , produciendo:
text
diff --git a/file.md b/file.md
--- a/file.md
+++ b/file.md
@@ -1,3 +1,5 @@
 # Title
+
trimToPatch conserva todas esas líneas (cada una coincide con PATCH_LINE). git apply --check --recount las recuenta como @@ -1,2 +1,3 @@: dos líneas de contexto, una adición de línea vacía. Si el contexto coincide en el archivo —y en un diff correcto es lo esperable— el parche aplica. El resultado es una línea vacía donde debería haber un bloque de código. La herramienta informa patch applies cleanly y muestra el --stat. El contenido quedó alterado y nadie lo nota.
No es solo un falso rechazo: es corrupción silenciosa. El camino de falso rechazo (contexto no coincide) también existe, pero el de corrupción es el que bloquea.
Corrección mínima: Anclar el cierre al inicio de línea: ```(?:diff|patch)?\n([\s\S]*?)^```$ con flag m, o como mínimo exigir \n``` (un salto de línea antes del cierre) para que los backticks en +```` no se confundan con un fence de cierre.
2. apply-files.js:42-43 — La regex de limpieza de etiquetas se traga contenido real cuando se pierde el fence
Fichero y línea: .claude/skills/multi-model-review/scripts/apply-files.js, líneas 42-43.
Qué garantía rompe: Que el fichero escrito es lo que el implementador produjo. El bucle while (seg.length && /^(Copy|Copiar|[a-z]{1,12})$/i.test(seg[0].trim())) seg = seg.slice(1); coincide con cualquier palabra de 1 a 12 letras, no solo con etiquetas de UI. El propio código documenta que los fences "may or may not survive" el renderizado. Si el fence de apertura se pierde, seg[0] es la primera línea del contenido real del fichero. Si esa línea es una palabra corta —import, export, pass, return, Hello, o cualquier palabra ≤12 letras— se elimina silenciosamente. El bucle while elimina todas las líneas consecutivas que coincidan, no solo una.
Escenario de fallo: El renderizador elimina el fence de apertura. El implementador produjo un Markdown cuyo primera línea es Importante. Tras trim(), Importante son 9 letras; coincide con [a-z]{1,12} (con flag i). Se elimina. La herramienta informa wrote file.md (N bytes) con N menor del correcto.
Corrección mínima: Solo eliminar etiquetas si la siguiente línea no vacía es un fence. Recorrer seg comprobando que hay un fence más adelante antes de descartar cada etiqueta; si no lo hay, conservar la línea.
3. apply-files.js:72-76 — El mensaje de error miente sobre la causa cuando parse descarta todos los ficheros
Fichero y línea: .claude/skills/multi-model-review/scripts/apply-files.js, líneas 72-76.
Qué garantía rompe: La fiabilidad del diagnóstico. parse() filtra ficheros con cuerpo vacío mediante .filter((f) => f.body.trim()). Si todos los ficheros del reply quedan vacíos —por el hallazgo 2, o porque el implementador produjo ficheros vacíos legítimos—, files.length === 0 y el error dice no "FICHERO: <ruta>" blocks found in ${REPLY}. Los bloques sí existían; el problema fue que parse los descartó. El operador re-asks al implementer por un incumplimiento del contrato que no ocurrió.
Corrección mínima: Distinguir los dos casos. Si marks.length > 0 && files.length === 0, reportar que se encontraron bloques pero todos quedaron vacíos tras el procesamiento.
4. collect.js:56-62 — !== null es código muerto; la lógica real es un try/catch disfrazado
Fichero y línea: .claude/skills/multi-model-review/scripts/collect.js, líneas 56-62.
Qué garantía rompe: Ninguna funcional, pero confunde al lector. execFileSync devuelve un Buffer, nunca null. La comparación !== null es siempre true. La detección de "behind" depende en realidad de que git merge-base --is-ancestor lance una excepción (exit 1) o no (exit 0). Un maintainer futuro podría leer !== null como la guarda, reorganizar el bloque y romper el try/catch sin darse cuenta.
Corrección mínima: Reescribir como try { execFileSync(...); behind = local !== prHead; } catch { /* not ancestor */ } para que la lógica sea explícita.
5. apply-files.js:91-103 — TOCTOU entre la validación de contención y la escritura
Fichero y línea: .claude/skills/multi-model-review/scripts/apply-files.js, líneas 91-103 (validación) vs. 121-130 (escritura).
Qué garantía rompe: La contención frente a un atacante activo. Entre fs.realpathSync(ancestor) y fs.mkdirSync + fs.openSync, un directorio intermedio existente puede ser reemplazado por un symlink que apunte fuera del repo. O_NOFOLLOW protege solo el componente final, no los intermedios. mkdirSync({ recursive: true }) sigue el symlink y crea directorios fuera del repo; openSync escribe ahí.
Corrección mínima: Re-validar con realpathSync el directorio padre inmediatamente antes de openSync, o usar fs.realpathSync en el directorio recién creado por mkdirSync.
6. apply-files.js:59 — Ficheros vacíos se descartan silenciosamente
Fichero y línea: .claude/skills/multi-model-review/scripts/apply-files.js, línea 59.
Qué garantía rompe: Que todo FICHERO: del reply se escribe. .filter((f) => f.body.trim()) descarta ficheros cuyo contenido es vacío o solo espacios. Un .gitkeep o un placeholder legítimo no se crea, y no hay error.
Corrección mínima: Separar "este bloque tiene contenido procesable" de "este fichero debe escribirse". Un cuerpo vacío es un fichero vacío válido.
Lo que no pude verificar
Tests: No tengo los ficheros de test en el diff. No puedo comprobar si algún test pasa por el motivo equivocado ni si cubren los caminos de fallo identificados. El issue afirma 49 tests offline; no puedo citar ni ejecutar ninguno.
gate.js, ask.js, build-prompt.js, roles.js: Fuera del alcance declarado de esta revisión. Las preguntas del issue sobre si el gate puede cerrar mal o si un fallo del revisor puede parecer limpio quedan sin responder desde este diff.
Frecuencia real de pérdida de fences: El hallazgo 2 depende de cuánto el renderizador dañe los fences. El código lo contempla, pero no puedo medir su frecuencia.
Supuestos
Asumo que apply-patch.js se usa con diffs de Markdown reales (companions, README, docs). Si ningún diff tocara nunca un archivo con bloques de código, el hallazgo 1 no se dispararía —pero el repo tiene Markdown con fences en abundancia.
Asumo que O_NOFOLLOW está disponible en el entorno de ejecución (Linux). En Windows no existe y el flag se coerce a 0, pero el código menciona /tmp y comportamiento Unix, así que asumo Linux.
Resumen
El hallazgo 1 es bloqueante: un diff que añada o elimine bloques de código en un Markdown —operación común en este repo— produce un parche truncado que git apply --recount puede aceptar como válido, aplicando contenido alterado sin error. Es exactamente la clase de defecto que más ha aparecido en este PR: éxito aparente sobre contenido corrupto. Los hallazgos 2 y 3 son la misma clase de problema en apply-files.js y conviene cerrarlos juntos.
