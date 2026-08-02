VEREDICTO: BLOQUEANTE

## Hallazgos

### Bloqueante — La protección contra symlinks sigue permitiendo escribir fuera del repositorio

Archivo: `.claude/skills/multi-model-review/scripts/apply-files.js:83-103`

Confirmo Qwen H1 y GLM B1.

La comprobación comienza en `path.dirname(target)`. Si el propio destino ya es un symlink hacia fuera, su directorio padre parece legítimo y `writeFileSync(target, body)` sigue el enlace.

Escenario: `repo/link.js -> /tmp/external.js`; una respuesta con `FICHERO: link.js` sobrescribe `/tmp/external.js`.

El test existente solo cubre un symlink de directorio (`enlace/robado.js`), no un symlink en el fichero final.

Corrección mínima: comprobar el destino existente mediante `lstatSync`/`realpathSync` y rechazar enlaces que salgan del repositorio. Para cerrar también la carrera entre validación y escritura, es preferible abrir el destino con `O_NOFOLLOW` y escribir mediante el descriptor. Añadir el caso de symlink final a la suite.

### Bloqueante — El procedimiento documentado no activa la comprobación de independencia

Archivo: `.claude/skills/multi-model-review/SKILL.md:91-112`

Confirmo Qwen H2 y GLM B2.

El paso 5 todavía afirma que `gate.js` no comprueba quién respondió y el paso 7 ejecuta el gate sin `--roles`. Por tanto, seguir el procedimiento oficial deja sin usar precisamente la protección añadida en este ciclo.

Además, `roles.js 197` sin `--dir` escribe `roles-pr197.json` en el directorio actual, no en `work/pr197/`. No basta con agregar al gate una ruta bajo `work/pr197`: el paso 1 debe ser, por ejemplo:

```bash
node scripts/roles.js 197 --dir work/pr197
```

y el gate:

```bash
node scripts/gate.js work/pr197/review-kimi.md \
  --cycle 5 --max-cycles 5 \
  --roles work/pr197/roles-pr197.json
```

Debe indicarse que `--roles` es obligatorio, no opcional.

### Medio — `apply-files.js` falla cuando la ruta del repositorio contiene un symlink

Archivo: `.claude/skills/multi-model-review/scripts/apply-files.js:83-88`

Confirmo GLM B3.

`repoReal` está canonicalizado, pero `target` se construye desde `REPO` sin canonicalizar. Para un workspace como `/workspace/repo -> /real/repo`, el destino `/workspace/repo/file` nunca empieza por `/real/repo/` y toda escritura se rechaza.

Corrección mínima:

```js
const target = path.resolve(repoReal, file);
```

Debe añadirse una prueba usando el repositorio mediante un symlink.

### Medio — Los argumentos inválidos del gate eluden el tope de ciclos

Archivo: `.claude/skills/multi-model-review/scripts/gate.js:25-34, 117-124`

Hallazgo adicional.

El contrato promete código 2 ante uso incorrecto, pero `--cycle cinco`, `--cycle` sin valor o `--max-cycles nope` producen `NaN`. La comparación `CYCLE >= MAX_CYCLES` resulta falsa y el proceso sale con código 1, invitando a continuar otro ciclo aunque el estado sea inválido.

Corrección mínima: exigir enteros positivos, `cycle <= max-cycles`, rechazar flags sin valor y salir 2 ante cualquier incumplimiento. Añadir pruebas para valores ausentes, no numéricos, cero y ciclo mayor que el máximo.

### Bajo — La persistencia de roles se puede eludir cambiando el directorio

Archivo: `.claude/skills/multi-model-review/scripts/roles.js:24-44`

Confirmo GLM O1, aunque queda parcialmente absorbido por el defecto documental anterior.

La asignación se fija únicamente dentro del fichero localizado por `DIR`, cuyo valor por defecto es el directorio actual. Ejecutar ciclos desde directorios distintos crea asignaciones independientes.

Corrección mínima: exigir `--dir` explícitamente o definir una ubicación canónica y documentada dentro del paquete de revisión.

### Bajo — Los temporales de `apply-patch.js` quedan abandonados en errores

Archivo: `.claude/skills/multi-model-review/scripts/apply-patch.js:84-136`

Confirmo Qwen H4 y GLM O2.

El directorio temporal solo se elimina después de una aplicación exitosa. Fallos de `git apply --check` o `git apply --3way` lo dejan atrás.

Corrección mínima: encerrar todas las operaciones posteriores a `mkdtempSync` en `try/finally`.

### Bajo — La atribución no está vinculada al PR revisado

Archivos: `.claude/skills/multi-model-review/scripts/ask.js:85-91`, `.claude/skills/multi-model-review/scripts/gate.js:42-68`

Confirmo parcialmente GLM O3.

El gate comprueba la familia, pero ni el metadato de la respuesta contiene el PR ni existe comparación con `roles.pr`. Pasar por error un fichero de roles de otro PR con el mismo árbitro no se detecta.

Corrección mínima: hacer que `ask.js` registre el número y commit del PR, y que el gate exija coincidencia con roles y con la información recolectada.

## Arbitraje de los demás puntos

- Qwen H3: no puedo confirmarlo como incumplimiento del objetivo. `ask.js` acepta `stdout` vacío, pero el objetivo atribuye el rechazo a los drivers hermanos, que no están incluidos. El gate lo trata de manera segura como revisión fallida. Conviene que `ask.js` también rechace salida vacía como defensa adicional.
- La observación de `REVIEW_PHRASE` de GLM es válida como fragilidad operativa, pero no bloqueante: entornos distintos producen un fallo cerrado, no una aprobación incorrecta.
- El refspec explícito de `collect.js` actualiza directamente la referencia usada para calcular el merge-base; no encuentro defecto en esa corrección.

## Preguntas y verificaciones no disponibles

El artefacto local contiene `c5/diff.patch`, contexto y revisiones, pero `c5/files/` está vacío y no existe un checkout Git. Por ello no pude ejecutar `node --test scripts/tests/contract.test.js`, revisar código adyacente fuera del diff ni consultar companions/ADRs adicionales. El intento de ejecutar la suite falló porque el fichero no está presente.

Tampoco están disponibles los drivers hermanos que supuestamente rechazan extracciones vacías. Por tanto, esa aceptación y la afirmación de “24 tests offline” no quedan verificadas independientemente.

## Resumen

Las correcciones de persistencia de roles y atribución están implementadas, pero el flujo oficial no las activa. La corrección de contención de rutas sigue siendo evadible mediante un symlink en el destino final y rompe workspaces cuyo repositorio se alcanza mediante symlink. Como este es el ciclo 5 y persisten bloqueantes, corresponde detener el ciclo y elevarlo al mantenedor; no continuar iterando ni aprobar el cambio.
