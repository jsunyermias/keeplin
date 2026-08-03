VEREDICTO: BLOQUEANTE

## Hallazgos

### Bloqueante — Una ejecución fallida de `ask.js` puede dejar vigente una revisión anterior y permitir que el gate la reutilice

Archivo: `.claude/skills/multi-model-review/scripts/ask.js`, líneas aproximadas 105–150.

`ask.js` escribe `<OUT>.raw`, pero antes de ejecutar no elimina ni invalida un `OUT` y un `OUT.meta.json` preexistentes. Si una ronda anterior produjo una revisión limpia y un reintento posterior falla —timeout, salida no cero o ausencia de veredicto—, los artefactos válidos de la ronda anterior permanecen intactos.

`gate.js` solo vincula esos artefactos al PR y al revisor, no al ciclo, prompt, diff ni commit. Por tanto, una revisión limpia de un estado anterior del mismo PR puede cerrar un ciclo posterior tras cambiar el código.

Corrección mínima:

- Invalidar o renombrar atómicamente `OUT` y `OUT.meta.json` antes de lanzar el driver.
- Escribir ambos mediante temporales y publicarlos solamente al completar todas las validaciones.
- Registrar y comprobar al menos la identidad inmutable del paquete revisado —por ejemplo, hash del prompt/diff o SHA del head— y el ciclo. Limpiar los artefactos evita el fallo inmediato, pero sin esa vinculación cualquier copia antigua del mismo PR sigue siendo reutilizable.

Este es exactamente el peor resultado señalado en el objetivo: un fallo del revisor conserva aspecto de revisión limpia.

### Bloqueante — `gate.js` permite cerrar sin `--roles`

Archivo: `.claude/skills/multi-model-review/scripts/gate.js`, líneas aproximadas 42–93.

Confirmo el hallazgo común de Qwen y GLM, y lo clasifico como bloqueante. La ayuda presenta `--roles` como obligatorio, pero toda la comprobación de independencia está condicionada por `if (ROLES)`. Esta invocación puede devolver 0:

```text
node gate.js review.md --cycle 1
```

Basta que el fichero tenga el veredicto limpio y la frase final; puede haberlo escrito el implementador o pertenecer a otro PR.

Corrección mínima: considerar la ausencia de `--roles` uso incorrecto y salir con 2. No debería existir un modo silencioso sin verificación de identidad.

### Bloqueante — `roles.js` puede asignar como árbitro a la misma familia por diferencias de mayúsculas

Archivo: `.claude/skills/multi-model-review/scripts/roles.js`, líneas aproximadas 53–87.

Confirmo el núcleo del hallazgo de GLM, con un escenario más grave. Los nombres se comparan literalmente. Para PR 197:

```text
node roles.js 197 --dir paquete Codex
```

`Codex` se considera implementador externo y, por paridad, se asigna `codex` como árbitro. Semánticamente es la misma familia juzgando su propio trabajo, aunque las cadenas sean diferentes. Lo mismo afecta a variantes con espacios o nombres de modelo que representen una familia rotativa.

Corrección mínima: normalizar identificadores conocidos (`trim().toLowerCase()`) antes de persistirlos y compararlos. Si se permiten nombres humanos libres, separar un identificador canónico de familia de un nombre descriptivo.

### Alta — `gate.js` interpreta valores de flags como el fichero posicional

Archivo: `.claude/skills/multi-model-review/scripts/gate.js`, línea aproximada 25.

Confirmo el hallazgo de GLM. `argv.find(a => !a.startsWith('--'))` no excluye los valores de `--cycle`, `--max-cycles` ni `--roles`.

Así, una forma razonable y sintácticamente válida como:

```text
node gate.js --cycle 1 --roles roles.json review.md
```

intenta juzgar el fichero `1`. Si ese nombre existe, evalúa contenido distinto del solicitado.

Corrección mínima: implementar un parser que consuma cada flag y su valor, exija exactamente un posicional y rechace argumentos desconocidos, duplicados o sobrantes.

### Alta — `ask.js` acepta un eco o cualquier valor como “veredicto real”

Archivo: `.claude/skills/multi-model-review/scripts/ask.js`, líneas aproximadas 126–139.

Confirmo el hallazgo de Qwen. El patrón acepta cualquier línea que empiece por `VEREDICTO:`, incluida la propia línea de opciones incluida en el prompt, un diagnóstico o `VEREDICTO: cualquier cosa`. Después publica `OUT` y su metadata, por lo que `build-prompt --prior` lo presenta al árbitro como una revisión real.

Corrección mínima: exigir exactamente uno de los tres valores permitidos y, conforme al prompt, exigirlo como primera línea. También conviene rechazar múltiples declaraciones de veredicto.

### Alta — `build-prompt.js` afirma falsamente que el cambio solo borra ficheros

Archivo: `.claude/skills/multi-model-review/scripts/build-prompt.js`, líneas aproximadas 181–200.

Confirmo el hallazgo común. Si todos los ficheros existentes se omiten por superar el presupuesto, `filesSection` queda vacío y `skipped` no, pero el mensaje afirma que el cambio solo borra ficheros. Además, no enumera los omitidos porque esa lista solo se añade cuando se incrustó al menos uno.

Corrección mínima: distinguir explícitamente:

- omisión por `--no-files`;
- omisión total por tamaño;
- cambio compuesto únicamente por borrados.

En el segundo caso debe enumerar `skipped`.

### Media — Un `collect.info` malformado desactiva silenciosamente su propia verificación

Archivo: `.claude/skills/multi-model-review/scripts/build-prompt.js`, líneas aproximadas 83–109.

Confirmo el hallazgo de ambos revisores. Si falta `files_captured`, el resultado es `NaN` y `Number.isInteger(captured)` evita toda comprobación. La mera existencia de `collect.info` no demuestra entonces que sea un paquete completo producido por `collect.js`.

Corrección mínima: exigir exactamente una línea válida `files_captured` con entero no negativo; cualquier ausencia, duplicado o valor inválido debe abortar.

### Media — Un objetivo solicitado pero inexistente se omite silenciosamente

Archivo: `.claude/skills/multi-model-review/scripts/build-prompt.js`, líneas aproximadas 112–114.

Confirmo el hallazgo de GLM. Si el operador proporciona `META` con un error tipográfico, el prompt se genera sin objetivo y sin advertencia. Esto contradice el comentario y el contrato de que el objetivo viaja en el prompt.

Corrección mínima: si se proporcionó `META`, exigir que sea un fichero legible; no tratar “inexistente” como “no solicitado”.

### Media — `gate.js` acepta una línea final que no es exacta

Archivo: `.claude/skills/multi-model-review/scripts/gate.js`, líneas aproximadas 125–130.

El contrato exige la frase como línea final exacta y sola, pero todas las líneas se pasan por `trim()`. Por ello una línea con espacios o tabuladores antes/después se acepta como exacta. También se ignoran líneas finales compuestas solo por espacios.

Corrección mínima: normalizar únicamente la terminación CRLF y comparar el contenido crudo de la última línea significativa según una regla documentada. Si “exactamente” es literal, no aplicar `trim()` a la línea de cierre.

### Media — JSON corrupto y otros errores de uso incumplen el contrato de salida 2

Archivo: `.claude/skills/multi-model-review/scripts/gate.js`, lecturas y `JSON.parse` de roles/metadata.

Un fichero de roles o metadata malformado provoca una excepción no capturada de Node, cuyo código habitual es 1. El contrato reserva 1 para “otra vuelta” y 2 para uso incorrecto. Un orquestador puede interpretar corrupción de artefactos como una revisión pendiente normal.

Corrección mínima: capturar errores de lectura y parseo, emitir un diagnóstico sin contenido sensible y salir con 2. Validar además la forma y tipos de ambos objetos, no solo que el JSON sea parseable.

### Observación — `ask.js --pr` sin valor falla demasiado tarde

Archivo: `.claude/skills/multi-model-review/scripts/ask.js`, líneas aproximadas 15–20 y 64–73.

Confirmo el hallazgo. `--pr` al final se convierte en ausencia de PR, ejecuta el costoso driver y publica una revisión sin vínculo al PR. Con `--roles`, el gate la rechazará después; sin `--roles`, el hallazgo bloqueante anterior permite incluso cerrarla.

Corrección mínima: si aparece `--pr`, exigir inmediatamente un entero positivo y rechazar duplicados.

### Observación — `build-prompt.js --out` sin valor y flags desconocidos no reciben validación coherente

Archivo: `.claude/skills/multi-model-review/scripts/build-prompt.js`, líneas aproximadas 30–45.

Confirmo `--out` sin valor: termina en un `TypeError` opaco. Además, `--no-files` cae también en `positional`, y los flags desconocidos se tratan como posicionales; actualmente puede funcionar por casualidad según el orden, pero el parser no establece una gramática fiable.

Corrección mínima: consumir expresamente cada flag, validar sus valores y rechazar duplicados, desconocidos y posicionales sobrantes.

## Hallazgos previos refutados o no demostrados

- `ctxDir` con subdirectorios: el fallo descrito por GLM es plausible, pero no demuestra una garantía contractual rota sin ver qué estructura produce `collect.js`. Si `context/` está definido como plano, sería robustez adicional. No lo elevo con la evidencia disponible.
- Que `Kimi` haga fallar posteriormente `ask.js` no es exactamente el problema: el riesgo demostrado es peor y está en `roles.js`, que puede considerar esa familia “externa” y asignarle su propia familia como árbitro.
- Qwen concluyó que no había ruta para que el gate cerrara incorrectamente. Lo refuto: `--roles` opcional y la reutilización de artefactos antiguos del mismo PR sí proporcionan rutas concretas.

## Preguntas y verificaciones no cubiertas

No pude ejecutar ni citar los 49 tests ni revisar sus nombres y aserciones: no se incluyeron en esta porción. Por tanto, no considero verificada la afirmación de cobertura por mutación.

`apply-files.js` está expresamente fuera de esta vuelta; no puedo determinar aquí si aún permite escapar del repositorio o rechaza rutas legítimas.

Tampoco pude contrastar `SKILL.md`, `collect.js`, `roles-pr197.json` ni la tabla verificable del issue con el estado local. Sus ausencias no son hallazgos de esta porción, pero limitan la verificación independiente.

## Resumen

La ronda no puede cerrarse. Persisten rutas por las que se puede aprobar sin comprobar independencia y, más grave aún, reutilizar una revisión limpia antigua después de que el intento correspondiente al estado actual haya fallado. También hay defectos confirmados de parsing e informes falsos que degradan directamente la evidencia entregada al árbitro.
