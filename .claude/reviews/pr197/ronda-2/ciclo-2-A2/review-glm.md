model: GLM-5.2
url: https://chat.z.ai/c/c21fcc7f-f4c1-479c-873a-bf8b6779ba58
--- reply ---
Show full message
Thought Process
VEREDICTO: OBSERVACIONES
Hallazgos
1. collect.js — el aviso de refs/pull/<n>/head desfasada solo va a stderr; collect.info no lo refleja
Impacto: medio — un paquete obsoleto es indistinguible de uno actual en el manifiesto que el pipeline lee.
collect.info registra head: <hash> pero no incluye ningún campo que indique que ese head va por detrás del checkout. El aviso viaja únicamente por stderr (console.error). Si un consumidor aguas abajo (build-prompt.js, fuera de alcance aquí) lee collect.info sin acceso al stderr de la ejecución de collect.js, no tiene forma de saber que el paquete describe un commit anterior. La corrección que se introdujo —detectar el desfase— queda a merced de que el canal stderr se capture.
Corrección mínima: añadir behind: true|false al manifiesto en collect.info, o escribir un fichero centinela (p. ej. STALE) en el directorio de salida cuando behind sea true, de modo que el pipeline lo detecte por fichero, no por stderr.
2. apply-files.js — replace(/\s*$/, '') elimina espacio final significativo, no solo el salto de línea
Impacto: bajo — corrupción silenciosa de contenido en un caso estrecho.
La normalización final hace seg.join('\n').replace(/^\n+/, '').replace(/\s*$/, ''). \s* al final de la cadena captura espacios y tabuladores, no solo newlines. Un fichero Markdown cuyo último renglón con contenido termine en dos espacios (salto de línea blando) los pierde silenciosamente, y la escritura se reporta como exitosa. El comentario dice «Exactly one trailing newline», pero el regex hace más que normalizar newlines.
Este comportamiento es pre-existente (el código anterior con replace(/\s+$/, '\n') tenía el mismo efecto), y la corrección de este ciclo era sobre el caso contrario (falta de salto de línea final). No obstante, el defecto sigue presente.
Corrección mínima: replace(/\n+$/, '') para preservar espacios y tabuladores del último renglón, y luego añadir \n.
3. apply-files.js — cerca exterior sin cerrar trunca contenido silenciosamente
Impacto: bajo — requiere que el modelo olvide cerrar la cerca exterior; el comportamiento mejoró respecto al código anterior, pero el defecto persiste en un caso distinto.
La búsqueda de la cerca de cierre itera desde el final y toma el último renglón que coincida con /^\s*```\s*$/. Si el modelo no cierra la cerca exterior, el último ``` interior (la cerca de cierre de un bloque de código del propio fichero) se toma como cierre exterior, y todo el contenido posterior se pierde. La escritura se reporta como exitosa.
Ejemplo: un fichero Markdown que contiene un bloque de código y texto después, sin cierre exterior:
text
FICHERO: ejemplo.md
Título
js
código
Más texto.
text
(sin cierre exterior)
El resultado es "# Título\n```js\ncódigo\n```\n" — «Más texto.» desaparece.
Corrección mínima: no hay corrección mínima sin cambiar el formato de transporte. Si la cerca exterior tiene etiqueta de lenguaje (```diff, ```js), exigrir que el cierre sea un ``` posterior al último cierre interior reduciría los falsos positivos. Alternativamente, contar las cercas: si el número de ``` es impar, la cerca exterior no se cerró, y conviene rechazar en vez de adivinar.
4. apply-files.js — LANGUAGE_LABELS no cubre etiquetas comunes; php, dockerfile, graphql, proto, etc.
Impacto: bajo — corrupción visible (línea espuria al inicio del fichero).
Si la interfaz de chat pone la etiqueta de lenguaje en un renglón separado encima de la cerca y la etiqueta no está en el conjunto cerrado, no se elimina y pasa a ser la primera línea del fichero. El conjunto cubre los lenguajes más frecuentes en este proyecto, pero la lista es incompleta para un uso general. La correlación con fenceFollows evita el sobre-borrado que tenía el código anterior, pero el bajo-borrado sigue produciendo contenido alterado.
Corrección mínima: añadir las etiquetas más comunes que falten, o documentar explícitamente que la lista es intencionalmente parcial y que las etiquetas no reconocidas se conservan como contenido.
5. apply-patch.js — la clase de caracteres de PATCH_LINE admite espacio, lo que permite prose con sangría dentro del parche recortado
Impacto: bajo — git apply --check rechaza en la mayoría de los casos, pero si la línea extra coincide con el fichero en esa posición, se acepta silenciosamente.
PATCH_LINE incluye [ +\-\\] como clase de caracteres válidos. Un renglón de prosa que empiece con un espacio pasa el filtro y se incluye en el parche entre el último hunk válido y la primera línea de prosa que no coincida. trimToPatch no lo excluye porque no es blanco. Si esa línea coincide con el contenido del fichero en esa posición, --recount la acepta como contexto.
Corrección mínima: después del último hunk, no extender last con líneas que coincidan únicamente con la clase de caracteres; requerir que sean diff --git, --- , +++ , @@ , , +, -, \ dentro de un hunk activo, no después.
Preguntas y supuestos
No puedo verificar los tests. El diff no incluye los ficheros de test. No puedo confirmar que los 72 tests pasan, ni que pasan por el motivo correcto, ni que cubren los caminos de fallo que afirman cubrir. Esto es lo que más me preocupa no haber podido verificar, dado el historial de este PR (tres tests que pasaban por el motivo equivocado).
No puedo verificar la interacción con build-prompt.js ni gate.js. Si build-prompt.js lee collect.info y no tiene acceso al stderr de collect.js, el hallazgo 1 se eleva a bloqueante: un paquete desfasado llegaría a los revisores sin ninguna marca. No puedo confirmarlo sin ver ese fichero.
No puedo verificar el comportamiento con \r\n. Los cuatro ficheros dividen por \n sin normalizar \r. Si la respuesta del modelo llega con finales de línea Windows, cada renglón conserva un \r que se incrusta en el contenido. En apply-files.js, el replace(/\s*$/, '') limpia el \r final, pero los \r intermedios sobreviven y se escriben al fichero. En apply-patch.js, git apply puede rechazar el parche por contexto con \r añadido. No sé si los drivers de chat que usa esta skill envían \r\n o \n.
No puedo verificar O_NOFOLLOW en todos los plataformas. El código asume comportamiento POSIX. Node.js en Windows puede no exponer O_NOFOLLOW o comportarse distinto. Si esta skill se ejecuta en Windows, el flag podría ser ignorado silenciosamente.
collect.js — el check de behind solo funciona si el checkout local está en la rama del PR. Si el operador cambia a main después de empujar y antes de recolectar, merge-base --is-ancestor prHead local falla (prHead no es ancestro de main), el catch pone behind = false, y el paquete desfasado no se detecta. No sé si este escenario ocurre en la práctica.
Resumen
Los cuatro ficheros están cuidadosamente implementados, con comentarios que describen tanto lo que defienden como lo que no defienden — un contraste notable con el estado anterior. No encontré forma de que apply-files.js escriba fuera del repositorio más allá del TOCTOU documentado con directorios intermedios, ni forma de que args.js acepte una invocación inválida como válida. apply-patch.js recorta el parche de manera razonable y git apply --check es una red de seguridad eficaz para la mayoría de los casos de contenido espurio.
El hallazgo más significativo es que collect.info no refleja el estado de desfase que collect.js detecta: la información viaja por stderr, no por el manifiesto, y un consumidor aguas abajo que solo lea ficheros no la vería. El resto son observaciones de bajo impacto sobre casos de borde en el parseo de cercas y etiquetas que son inherentes al formato de transporte basado en texto.
