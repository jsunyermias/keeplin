VEREDICTO: BLOQUEANTE

## Hallazgos

### BLOQUEANTE — `apply-files.js` permite escribir fuera del repositorio mediante symlinks

Fichero: `scripts/apply-files.js:83-98`.

La validación usa únicamente `path.resolve()`. Una ruta como `repo/enlace/fichero`, donde `enlace` es un symlink a un directorio externo, pasa la comprobación textual y `writeFileSync()` sigue el enlace. Un implementador o una respuesta manipulada puede sobrescribir archivos fuera del repositorio.

Esto contradice explícitamente la garantía documentada de que el script “refuses to write outside the repository” y puede causar pérdida de datos.

Corrección mínima: resolver con `realpath` el repositorio y cada ancestro existente del destino, rechazar cualquier symlink que salga de la raíz y abrir el fichero evitando seguimiento de symlinks cuando sea posible. Añadir un test con un symlink interno dirigido a un directorio externo.

### BLOQUEANTE — el implementador no queda realmente fijo durante la vida del PR

Ficheros: `scripts/roles.js:21-35`, `scripts/tests/contract.test.js:95-102`.

`roles.js 197` devuelve Codex, pero `roles.js 197 kimi` devuelve Kimi. Nada persiste ni valida la asignación anterior. Por tanto, entre ciclos puede cambiarse el implementador usando el argumento documentado como override. El test ejecuta dos veces exactamente el mismo comando sin override, por lo que solo demuestra determinismo, no la invariancia declarada.

Escenario: Codex implementa el ciclo 1; en el ciclo 2 se invoca el override Kimi. Codex pasa entonces a ser adjudicador de un diff acumulado que contiene su propio trabajo.

Corrección mínima: persistir la asignación inicial en los metadatos del PR y rechazar overrides posteriores incompatibles. El test debe crear una asignación, intentar cambiarla y exigir fallo.

### ALTA — el gate no vincula el resultado con la familia asignada

Ficheros: `scripts/ask.js`, `scripts/gate.js:28-80`.

Confirmo Qwen OBS-1. `gate.js` acepta cualquier archivo que contenga las dos señales, sin probar que procede del adjudicador calculado por `roles.js`. La propia documentación reconoce que “gate.js does not check who replied”. Esto deja una regla central de independencia enteramente en manos de copy-paste manual.

Combinado con el problema anterior, puede cerrar una revisión realizada por el implementador.

Corrección mínima: generar un manifiesto por ciclo con PR, implementador, adjudicador y revisores; `ask.js` registra la identidad efectiva y `gate.js` exige coincidencia. Añadir un test que pase una revisión limpia atribuida al implementador y espere exit 2.

### MEDIA — la corrección del fetch no está verificada por el test presentado

Fichero: `scripts/tests/contract.test.js:294-329`.

Confirmo GLM O1. El setup ejecuta `git fetch origin` después de publicar `main`, por lo que `origin/main` ya está actualizado. Si se elimina de `collect.js` el refspec nuevo de la base, ambos tests continúan comprobando solamente presencia de `AGENTS.md` y éxito, no que el merge-base, diff o tracking ref sean actuales.

Esto invalida la afirmación concreta de que revertir cada corrección hace fallar sus tests.

Corrección mínima: dejar deliberadamente obsoleto `origin/main`, avanzar `main` en el remoto sin fetch y comprobar `collect.info`, `diff.patch` y `changed-files.txt`.

### MEDIA — `apply-files.js` puede alterar silenciosamente contenido sin fence

Fichero: `scripts/apply-files.js:47-65`.

Confirmo Qwen OBS-2 y GLM O3. La expresión `[a-z]{1,12}` elimina cualquier primera línea formada por una palabra corta. Si falta el fence, un fichero que empiece por `import`, `export` o `return` se escribe sin esa línea y se informa como éxito.

Que la respuesta incumpla el formato no autoriza a modificarla silenciosamente: debe rechazarse.

Corrección mínima: exigir fences o limitar el stripping a etiquetas UI conocidas y solo cuando inmediatamente después exista un fence.

### MEDIA — el soporte declarado de parches binarios sigue incompleto

Fichero: `scripts/apply-patch.js:44-65`.

Confirmo GLM O4 y amplío Qwen OBS-3. Reconocer `GIT binary patch` no basta: las líneas `literal N`/`delta N` y el payload base85 posterior no satisfacen `PATCH_LINE`, por lo que el parche se trunca. La afirmación de que las cabeceras binarias ya están soportadas es incorrecta.

Corrección mínima: parsear por estructura de diff en vez de una lista incompleta de prefijos, o soportar explícitamente bloques binarios completos. Añadir un test generado mediante `git diff --binary` y aplicarlo byte por byte.

### BAJA — directorios temporales quedan abandonados en varios caminos

Fichero: `scripts/apply-patch.js:85-134`.

Confirmo GLM O2. El directorio solo se elimina después de una aplicación exitosa. Fuga en:

- fallo de ambos `git apply --check`;
- modo `--check`, que retorna antes del cleanup;
- fallo de `git apply --3way`.

Corrección mínima: envolver todo lo posterior a `mkdtempSync()` en `try/finally`.

### BAJA — documentación inexacta sobre el presupuesto

Fichero: `scripts/build-prompt.js:106-113`.

Confirmo GLM O5. `AGENTS.md` nunca se omite, pero su tamaño sí incrementa `ctxUsed` y puede excluir todos los recursos opcionales. “Exempt from the budget” no describe ese comportamiento.

Corrección mínima: indicar que el recurso requerido nunca se descarta, pero consume el presupuesto disponible para recursos opcionales.

## Arbitraje de las restantes observaciones

- Qwen OBS-4, límite de ~70 KB: confirmada como riesgo operativo. No produce falso positivo porque el gate falla cerrado, pero la selección sigue siendo manual y puede desperdiciar ciclos.
- Qwen OBS-5, exigir que `OUT` esté fuera del repositorio: no lo confirmo como defecto contractual. Un directorio ignorado dentro del repositorio puede ser una elección válida; lo necesario sería documentarlo o comprobar que no pueda terminar staged accidentalmente.
- Qwen OBS-3 sobre tests de modo/copia: confirmada como carencia de regresión. El caso binario es además un defecto actual, no solo cobertura futura.

## Verificación y supuestos

- El issue completo y su tabla de verificación no estaban disponibles; solo `meta.md`. Tampoco había ADR o companion aplicable en el paquete entregado.
- Intenté ejecutar los 20 tests. Ninguno pudo completarse porque este entorno es de solo lectura y todos crean recursos temporales; el error fue `EROFS` al usar `mkdtemp`. Por tanto, no considero verificado el check offline.
- La corrección de `collect.js` y la inclusión obligatoria de `AGENTS.md` son correctas por inspección, pero la primera carece del test de regresión afirmado.
- No pude ejecutar los drivers web ni verificar sesiones/modelos externos.

## Resumen

Las correcciones del ciclo 3 mejoran el cambio, pero el contrato todavía no queda satisfecho. Hay dos bloqueantes: escritura fuera del repositorio mediante symlinks y ausencia de una asignación inmutable del implementador. Además, el gate no autentica la familia revisora y la afirmación de cobertura por mutación del fetch no se sostiene. No debe cerrarse este ciclo todavía.
