VEREDICTO: BLOQUEANTE

## Hallazgos propios

### BLOQUEANTE — Las opciones desconocidas se ignoran y un `--check` mal escrito realiza la escritura

- `.claude/skills/multi-model-review/scripts/apply-files.js:17-19`
- `.claude/skills/multi-model-review/scripts/apply-patch.js:17-19`
- `.claude/skills/multi-model-review/scripts/collect.js:21-33`

Los tres parsers eliminan cualquier argumento que empiece por `--`, aunque no sea una opción reconocida. Por ejemplo:

```text
node apply-files.js repo reply --chek
node apply-patch.js repo reply --chek
```

Ambas invocaciones parecen solicitar comprobación, pero ejecutan la mutación real porque `CHECK` es falso y `--chek` desaparece silenciosamente del conjunto posicional. También se aceptan opciones desconocidas y argumentos posicionales sobrantes.

Esto viola el contrato de uso y convierte un error tipográfico en una escritura o aplicación irreversible.

Corrección mínima: parser estricto que consuma exclusivamente las opciones declaradas, compruebe la aridad exacta y rechace flags desconocidos, duplicados o con valores inválidos. Añadir pruebas específicas con `--chek`, `--unknown` y un cuarto posicional, verificando que el árbol permanece intacto.

### BLOQUEANTE — Un diff cercado se trunca en los backticks añadidos a un Markdown y puede aplicarse como éxito

- `.claude/skills/multi-model-review/scripts/apply-patch.js:24-28`

Confirmo el hallazgo 1 de GLM. El cierre de:

```js
/```(?:diff|patch)?\n([\s\S]*?)```/g
```

no está anclado a una línea de cierre. Una línea de diff `+````, habitual al añadir bloques Markdown, contiene la secuencia que termina la captura. `trimToPatch()` puede conservar el `+` anterior a esos backticks y `--recount` puede transformar el fragmento en un parche válido pero distinto.

Es corrupción silenciosa: el comando puede salir correctamente después de aplicar solo una fracción de la respuesta.

Corrección mínima: reconocer fences por líneas completas y emparejar el cierre exterior, no por una subcadena. Debe existir una prueba que añada y elimine bloques cercados en Markdown y compare el fichero resultante byte por byte con el esperado.

### ALTA — `apply-files` elimina contenido real cuando no sobrevive el fence

- `.claude/skills/multi-model-review/scripts/apply-files.js:47-49`

Confirmo Qwen H-4 y GLM 2. `[a-z]{1,12}` no identifica chrome: identifica cualquier primera línea formada por una palabra corta. Sin fence, líneas reales como `import`, `export`, `package`, `Hello` o `Importante` se eliminan; el bucle puede eliminar varias consecutivas.

Corrección mínima: eliminar únicamente etiquetas cerradas conocidas cuando se haya demostrado que preceden a un fence. En la ruta sin fence no debe retirarse ninguna línea como supuesto chrome. Probar fidelidad byte a byte con primeras líneas de una y varias palabras cortas.

### ALTA — `collect.js` reutiliza contenido residual de paquetes anteriores

- `.claude/skills/multi-model-review/scripts/collect.js:51`
- `.claude/skills/multi-model-review/scripts/collect.js:127-138`
- `.claude/skills/multi-model-review/scripts/collect.js:140-161`

Confirmo Qwen H-1. `files/` y `context/` se crean sin limpiar el contenido previo. Al reutilizar `OUT`, archivos capturados en otra área, otro commit o incluso otro PR sobreviven aunque ya no aparezcan en `changed-files.txt`. `files_captured` cuenta solamente las escrituras actuales y, por tanto, puede contradecir el árbol entregado.

Esto puede exponer contenido fuera del alcance declarado o introducir contexto obsoleto en una revisión.

Corrección mínima: construir todo el paquete en un directorio temporal nuevo y renombrarlo atómicamente sobre el destino, o rechazar un `OUT` no vacío. Limpiar solo `files/` al principio evita residuos, pero deja además paquetes parciales si una operación posterior falla.

### MEDIA — Los errores dejan un paquete parcial con apariencia reutilizable

- `.claude/skills/multi-model-review/scripts/collect.js:51-161`

`diff.patch` y `changed-files.txt` se escriben antes de verificar `AGENTS.md`; los archivos completos también pueden haberse copiado antes de ese fallo. En otras rutas, un error de `git show` se interpreta indiscriminadamente como borrado. Tras fallar, queda un directorio que contiene parte de la estructura normal de un paquete.

Aunque la versión descrita de `build-prompt.js` exige `collect.info`, otras herramientas o una ejecución posterior sobre el mismo `OUT` pueden mezclar esos residuos. Es la misma raíz operacional del hallazgo anterior.

Corrección mínima: ensamblaje transaccional en un directorio temporal y publicación únicamente después de completar todas las verificaciones.

### MEDIA — Los ficheros vacíos se omiten y el diagnóstico niega bloques que sí existían

- `.claude/skills/multi-model-review/scripts/apply-files.js:68-82`

Confirmo GLM 3 y 6. `.filter((f) => f.body.trim())` descarta bloques vacíos o compuestos por espacios. Si todos son vacíos, el mensaje asegura que no se encontró ningún bloque `FICHERO`, aunque sí se encontró.

Un fichero vacío es un contenido completo válido; además, omitir solo algunos bloques produce éxito parcial sin aviso.

Corrección mínima: preservar los bloques vacíos y separar “no hubo cabeceras” de “hubo un bloque mal formado”. Añadir pruebas tanto para un único fichero vacío como para una respuesta que mezcle uno vacío y uno no vacío.

### MEDIA — La contención no resiste sustitución concurrente de directorios intermedios

- `.claude/skills/multi-model-review/scripts/apply-files.js:105-139`
- `.claude/skills/multi-model-review/scripts/apply-files.js:147-159`

Confirmo GLM 5 bajo un modelo con otro proceso capaz de modificar el workspace. `O_NOFOLLOW` protege exclusivamente el componente final. Después de validar el ancestro, un directorio intermedio puede sustituirse por un symlink; tanto `mkdirSync({recursive:true})` como `openSync(target, …)` lo siguen.

La corrección propuesta por GLM —revalidar inmediatamente antes de `openSync`— reduce la ventana pero no la cierra. Para una garantía frente a carreras se necesita resolución basada en descriptores (`openat`/`openat2`, `O_DIRECTORY`, `O_NOFOLLOW`, `RESOLVE_BENEATH`) o aislamiento que impida modificaciones concurrentes. Si ese atacante queda fuera del modelo, debe documentarse explícitamente; el comentario actual presenta las “tres partes” como contención completa.

## Arbitraje de los demás hallazgos previos

- Qwen H-2, comentario de `--3way`: **confirmado como observación documental**. El flujo normal solo llega a `--3way` tras superar una aplicación limpia con `--check`; no respalda el escenario de deriva que comenta `apply-patch.js:143-145`.
- Qwen H-3 / GLM 4, comparación `!== null`: **confirmado como observación de claridad**. `execFileSync` devuelve un valor en éxito y lanza en fallo; la comparación no aporta ninguna decisión.
- Sobre la aparente diferencia entre Qwen y GLM respecto de `apply-files`: no hay contradicción sustantiva. Qwen consideró sólida la contención sin atacante concurrente; GLM examinó una carrera con un segundo actor. La contención estática frente a rutas y symlinks preexistentes sí se sostiene, pero no la garantía frente a sustitución concurrente.
- GLM 1 frente al resumen más favorable de Qwen sobre `apply-patch`: **se sostiene GLM**. El parser cercado permite demostrar una alteración silenciosa antes de que intervenga `git apply`.

## Tests y verificaciones

Los tests suministrados cubren symlinks preexistentes, repositorios alcanzados mediante symlink, conservación de fences internos en `apply-files`, parches con recuentos incorrectos, contexto perdido, creación/borrado/rename y varias rutas de `collect`.

No cubren las garantías indicadas arriba:

- typo o flag desconocido en ninguno de estos CLI;
- diff cercado que modifica fences Markdown;
- contenido sin fence que comienza por palabras cortas;
- ficheros vacíos;
- reutilización del mismo `OUT`;
- ausencia de residuos tras fallar `collect`;
- sustitución concurrente de un directorio intermedio.

No pude ejecutar los 49 tests como suite: el paquete disponible separa los scripts en `A/files` y el test en `C/files`, de modo que su resolución relativa no representa el árbol completo del repositorio. Tampoco se aportó una tabla de verificación del issue distinta del relato de `meta.md`; por ello no puedo citar resultados de CI ni validar la afirmación de mutación.

## Resumen

Hay dos bloqueantes de integridad: un typo de `--check` ejecuta cambios reales y el extractor de parches puede aplicar silenciosamente un diff truncado al modificar Markdown cercado. También permanecen pérdida de contenido en `apply-files`, paquetes residuales/parciales en `collect`, manejo incorrecto de ficheros vacíos y una contención que no cubre carreras sobre directorios intermedios. No procede cerrar esta ronda.
