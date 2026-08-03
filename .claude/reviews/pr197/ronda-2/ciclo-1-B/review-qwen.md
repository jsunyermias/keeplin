model: Qwen3.8-Max-Preview
url: https://chat.qwen.ai/c/dc4b3c6a-3ac9-406f-a73f-698d53004cc1
--- reply ---
# 0.C — Revisión independiente y adversarial
Revisa el cambio como una persona o familia de modelo distinta de quien lo implementó. Tu
fuente primaria es el objetivo y criterios del issue junto con el diff y el estado actual
del repositorio; la explicación del autor es contexto, no prueba.
## Método
1. Lee `AGENTS.md`, el issue, ADRs aplicables y companions relevantes.
2. Reconstruye el contrato esperado y traza cada criterio de aceptación hasta código,
   documentación y tests, partiendo de la tabla de verificación del issue. Una fila cuyo
   verificador no puedas ejecutar ni citar es un hallazgo, no un detalle de forma.
3. Revisa el diff completo y el código adyacente. Busca especialmente:
   - pérdida, corrupción, duplicación o resurrección de datos;
   - autenticación, autorización, revocación, secretos y exposición de contenido;
   - carreras, reintentos, idempotencia, orden, backpressure y desconexiones;
   - compatibilidad cross-repo, versiones, pines y formatos persistentes;
   - migración, rollback, recuperación, observabilidad y operación multi-réplica;
   - divergencia entre `.rs`, companion, Graphify, README, SECURITY, RUNBOOK o ADR;
   - tests que pasan sin demostrar la garantía o que omiten caminos de fallo.
4. Verifica los checks disponibles de forma independiente. No conviertas una casilla del
   autor en evidencia de CI.
5. Clasifica cada hallazgo por impacto, señala archivo y línea, explica el escenario de
   fallo y propone la corrección mínima. No escondas bloqueantes en un resumen narrativo.
Si revisas un cambio ya fusionado para saldar una entrada de `docs/review-debt.md`, aplica
este mismo método sobre el diff fusionado, abre los hallazgos como issues por su propio valor
y mueve la entrada a `Cleared` enlazando tu revisión. Una relectura de la familia que
implementó no salda nada.
## Salida
Entrega primero los hallazgos ordenados por severidad, luego preguntas o supuestos y por
último un resumen breve. Si no hay hallazgos, dilo explícitamente y enumera riesgos no
cubiertos o verificaciones que no pudiste ejecutar. No apruebes por ausencia de pruebas en
contra y no hagas merge.
---
## Objetivo del cambio
PR #197 — `.claude/skills/multi-model-review/`
## Qué es esto
Una skill que hace pasar un pull request por revisores de varias familias de
modelos. Qwen y GLM revisan siempre y a ciegas, sin verse entre sí; el tercer
asiento lo ocupa quien de Kimi o Codex no implementó. Los diffs y las
revisiones viajan como ficheros para que casi nada pase por el contexto del
agente que orquesta.
`gate.js` es lo único que cierra una revisión: exige veredicto `SIN HALLAZGOS`
**y** la frase mágica como única última línea, y devuelve 0 (cerrado), 1 (otra
vuelta) o 2 (uso incorrecto o tope de ciclos).
Este PR lo implementó Claude, fuera de la rotación. Está registrado así en
`roles-pr197.json`, y por eso el árbitro es Codex y el gate puede correr con
`--roles` de verdad.
## En qué punto estás
Esta es una **ronda nueva**, con el contador a cero. La anterior llegó al tope
de 5 ciclos con bloqueantes vivos y se detuvo, como manda el contrato. Todos
los bloqueantes que quedaron abiertos allí están corregidos en este diff, cada
uno reproducido antes de tocar nada y cada corrección verificada por mutación:
revertirla hace fallar su test. Los tests son 49 y corren offline.
Lo corregido desde entonces:
1. `apply-files.js` tenía la contención mal en las **dos** direcciones. La
   comprobación arrancaba en el directorio padre del destino, así que si el
   destino *era* el symlink, el padre parecía legítimo y la escritura salía del
   repositorio. Y los destinos se resolvían contra el argumento mientras el
   repositorio se canonicalizaba, así que un workspace alcanzado por symlink
   rechazaba toda escritura legítima. Ahora los destinos se resuelven contra la
   ruta real, un último componente que sea symlink se rechaza apunte a donde
   apunte, y la escritura pasa por `O_NOFOLLOW`.
2. `gate.js` convertía un `--cycle` no numérico en `NaN`; como toda comparación
   con `NaN` es falsa, se saltaba el tope y salía 1 («otra vuelta») ante una
   invocación que no sabía leer. Ahora los argumentos inválidos son uso
   incorrecto.
3. `gate.js` solo comparaba la familia del revisor, de modo que un fichero de
   roles de otro PR con el mismo árbitro cerraba este. `ask.js` registra `--pr`
   y el gate exige que coincida.
4. `build-prompt.js` informaba de `changed-files.txt` en vez de lo que llegó a
   incrustar. Así fue como el ciclo 5 mandó a tres revisores un prompt sin
   contexto de ficheros mientras registraba «9 files». Ahora rechaza un paquete
   que no produjo `collect.js`, rechaza uno al que le falten ficheros que
   `collect.js` sí capturó, avisa en el propio prompt cuando no viaja texto
   completo, y cuenta lo que incrustó.
5. `roles.js` no sabía expresar un implementador de fuera de la rotación —que
   es justo lo que tiene este PR—, así que la comprobación de independencia
   corría desactivada en el único caso para el que se escribió. Ahora lo
   registra, elige árbitro no conflictuado por paridad, y exige `--dir`.
6. `SKILL.md` documentaba un procedimiento que nunca pasaba `--roles` y seguía
   afirmando que el gate no comprobaba quién respondía.
7. `collect.js --area` y `build-prompt.js --no-files` son nuevos: la
   documentación mandaba partir un PR grande por áreas y quitar el texto
   completo de los ficheros, y no daba con qué hacer ninguna de las dos cosas.
   La partición se hacía a mano, y un paquete hecho a mano llegó a tres
   revisores con `files/` vacío.
8. `ask.js` ya no deja una ejecución fallida con aspecto de revisión. Escribía
   lo que llegase directamente en la ruta del revisor y decidía después si el
   run había fallado; en la ronda de revisión de este mismo PR, una sesión de
   GLM atascada dejó 15 bytes de banner del driver en `review-glm.md`. El gate
   se habría salvado porque además exige `.meta.json`, pero
   `build-prompt --prior` adjunta las revisiones previas por ruta y le habría
   pasado ese banner al árbitro como si fuera la opinión de un revisor. Ahora
   la respuesta cruda va a `<out>.raw` y la ruta del revisor solo se escribe si
   la respuesta lleva línea de veredicto. Comprobar solo que no esté vacía deja
   pasar un banner o una respuesta truncada, y un revisor que falló pero parece
   decir «he mirado y no hay nada» es el peor resultado de esta pieza.
9. `collect.js` avisa cuando `refs/pull/<n>/head` va por detrás del checkout.
   GitHub actualiza esa referencia un poco después del push, así que recolectar
   justo después de empujar da un paquete coherente que describe el commit
   anterior, y toda la revisión juzga código que no es el que se escribió. Pasó
   preparando esta misma ronda.
Al escribir los tests aparecieron tres defectos más, también corregidos: el
filtro de argumentos posicionales descartaba `argv[0]` cuando su flag faltaba
(el `roles.js 197` documentado nunca había funcionado); `apply-files`
normalizaba el espacio final con una regla que solo se disparaba si había
espacio final, escribiendo ficheros sin salto de línea; y el helper de tests
descartaba stderr en las ejecuciones correctas, así que cualquier aserción
sobre lo que informaba una ejecución con éxito comparaba contra la cadena
vacía y pasaba por el motivo equivocado.
## Qué te pido
Que busques lo que siga roto, no que confirmes lo anterior. En concreto:
- ¿Queda alguna forma de que `apply-files.js` escriba fuera del repositorio, o
  de que rechace algo legítimo?
- ¿Puede `gate.js` cerrar un ciclo que no debería cerrarse, o dejar pasar una
  invocación inválida como si fuese válida?
- ¿Hay alguna otra ruta por la que un fallo del revisor pueda confundirse con
  una revisión limpia? Es el peor resultado posible para esta pieza.
- ¿Miente algún contador, mensaje o comentario sobre lo que el código hace de
  verdad? Ese es el defecto que más veces ha aparecido aquí.
- ¿Algún test pasa por el motivo equivocado, o cubre menos de lo que su nombre
  promete? Ya ha ocurrido tres veces en este PR.
Si algo no puedes verificar con lo que te doy, dilo en vez de suponerlo.
---
## Contrato del proyecto
### AGENTS.md
# Keeplin agent guide
This file is the provider-neutral, canonical contract for every human or automated agent
working in either Keeplin repository. Read it completely before changing code or
documentation. Provider-specific files such as `CLAUDE.md` may add tool setup, but must not
repeat or override this contract.
## What Keeplin is and where work belongs
Keeplin is a pre-release, self-hostable notes system written in Rust. The repositories split
responsibility as follows:
- `jsunyermias/keeplin`
  - `keeplin-core`: domain models, storage backends, sync, collaboration client, shared wire
    and format contracts.
  - `keeplin-daemon`: gRPC and REST surfaces, configuration, authentication, metrics and
    local process lifecycle.
- `jsunyermias/keeplin-srv`
  - `crates/keeplin-srv`: the Axum/PostgreSQL multi-user server, collaborative WebSocket
    sessions, sync relay, accounts, devices and sharing.
Put shared types and constants in `keeplin-core`; the server consumes them from a pinned
revision. A change that crosses the repository boundary is one logical change and needs
coordinated PRs and contract tests.
## Commands
Run commands from the root of the repository being changed.
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check-docs.sh
```
For `keeplin`, the focused suites are `cargo test -p keeplin-core` and
`cargo test -p keeplin-daemon`. For `keeplin-srv`, `cargo test --workspace` must run
against PostgreSQL and includes the `sqlx::test` integration tests described in its README.
CI builds the knowledge graph with the pinned version and publishes `graphify-out/` as a
workflow artifact. To reproduce that build and its validation locally, run:
```bash
pip install graphifyy==0.9.25
GRAPHIFY_REQUIRED=1 ./scripts/check-graph.sh
```
The generated `graphify-out/` directory is ignored and must never be staged or committed. The
old auto-refresh pre-commit hook was removed because CI, rather than a commit, now owns the
artifact. I
Pensamiento completado
VEREDICTO: OBSERVACIONES
Hallazgos
M-1 — build-prompt.js línea ~175: el mensaje al revisor miente cuando todos los ficheros se omiten por tamaño
Fichero: build-prompt.js, bloque else if (changed.length) (líneas ~170-183 del diff).
Qué rompe: La garantía de que el prompt dice la verdad sobre lo que el revisor no tiene. Es la clase exacta de defecto que el PR identifica como el más recurrente.
Escenario: Un PR toca tres ficheros de 50 KB cada uno. FILES_BUDGET_BYTES es 120 KB. El primero se incrusta, el segundo y el tercero van a skipped. filesSection no está vacío, así que se lista lo omitido: correcto. Pero si los tres ficheros superan individualmente el presupuesto (o la suma acumulada lo agota antes del primero), filesSection queda vacío, skipped tiene tres entradas, NO_FILES es false, y el prompt dice:
«No se adjunta el texto completo de ningún fichero: este cambio solo borra ficheros.»
Los ficheros no fueron borrados; existen en files/ y se omitieron por tamaño. El revisor juzga con una explicación falsa de por qué no tiene contexto, y calibra mal la fiabilidad de sus propias conclusiones.
Corrección mínima: Distinguir el caso skipped.length > 0 && !NO_FILES del caso «todos borrados». Por ejemplo:
js
1
2
3
4
5
NO_FILES
  ? '…omitido a propósito…'
  : skipped.length
    ? `No se adjunta el texto completo de los ficheros: todos superan el presupuesto de ${FILES_BUDGET_BYTES / 1024} KB. Juzga solo por el diff.`
    : 'No se adjunta el texto completo de ningún fichero: este cambio solo borra ficheros. Juzga únicamente por el diff.',
M-2 — ask.js línea ~131: cualquier línea VEREDICTO: pasa el filtro, incluido un eco de las instrucciones del prompt
Fichero: ask.js, const verdictLine = (stdout.match(/^VEREDICTO:.*$/m) || [])[0];
Qué rompe: La garantía de que un fichero en la ruta del revisor (con su .meta.json) es una revisión real y no un fallo del modelo que produjo salida estructuralmente válida pero vacía de contenido.
Escenario: El prompt que construye build-prompt.js contiene literalmente la línea:
1
VEREDICTO: BLOQUEANTE | VEREDICTO: OBSERVACIONES | VEREDICTO: SIN HALLAZGOS
Un modelo que no engage (truncamiento, negativa, eco del prompt) puede devolver esa línea como parte de su salida. El driver termina con exit 0, stdout no está vacío, y el regex /^VEREDICTO:.*$/m encuentra la línea de instrucciones. ask.js escribe OUT y .meta.json. El gate no cierra (el veredicto normalizado no es SIN HALLAZGOS), pero build-prompt --prior adjunta ese fichero al prompt del árbitro como si fuera la opinión de un revisor. El árbitro recibe una «revisión» que es una línea de instrucciones, no un juicio.
Es la misma familia del banner de 15 bytes que motivó el cambio, con la diferencia de que ahora pasa el filtro porque el prompt siembra la cadena que el filtro busca.
Corrección mínima: Validar que el veredicto es uno de los tres esperados:
js
1
2
3
4
5
6
7
const VALID = ['BLOQUEANTE', 'OBSERVACIONES', 'SIN HALLAZGOS'];
const verdictValue = (verdictLine || '').replace(/^VEREDICTO:\s*/, '').trim().toUpperCase();
if (!stdout.trim() || !VALID.includes(verdictValue)) {
  throw new Error(
    `${REVIEWER} produced no valid verdict line (${Buffer.byteLength(stdout)} bytes). …`
  );
}
B-1 — gate.js: --roles es opcional; sin él, no hay ninguna comprobación de identidad
Fichero: gate.js, línea ~73: if (ROLES) { … } envuelve toda la verificación de quién revisó.
Qué rompe: La garantía de que quien cierra el ciclo es el árbitro designado y no el implementador u otro agente. El propio comentario dice «Whose review is this?» y luego hace la respuesta opcional.
Escenario: node gate.js review.md --cycle 1 sin --roles. Cualquier fichero con VEREDICTO: SIN HALLAZGOS en la primera línea y la frase mágica como última línea cierra el ciclo. El implementador puede escribir ese fichero directamente. La documentación (SKILL.md) puede mandar pasar --roles, pero el gate no lo exige; la defensa depende de que el operador no se salte un paso que la herramienta no impone.
No lo clasifico como bloqueante porque el flujo documentado sí pasa --roles, y el gate es una herramienta interna cuyo invocador es el propio pipeline. Pero es un hueco de defensa en profundidad: la única pieza que puede cerrar una revisión no requiere la única comprobación que impide que el implementador se autoapruebe.
Corrección mínima: Hacer --roles obligatorio (pasar a bad si falta), o al menos emitir un aviso en stderr y exigir una variable de entorno (GATE_ALLOW_NO_ROLES=1) para ejecutar sin él.
O-1 — ask.js: --pr sin valor no produce error; el fallo aparece solo en el gate
Fichero: ask.js, línea ~17: const PR = prAt === -1 ? undefined : argv[prAt + 1];
Si --pr es el último argumento, PR queda undefined, no se lanza error, y .meta.json se escribe sin campo pr. El gate con --roles rechaza después (meta.pr === undefined), pero entre medias ask.js ha escrito OUT, .raw y .meta.json con aspecto de revisión válida. El operador ve el veredicto en stdout y solo descubre el problema al intentar cerrar. Es un fallo diferido, no un agujero, pero contradice el principio del propio PR: «ask.js ya no deja una ejecución fallida con aspecto de revisión».
Corrección: Si prAt !== -1 y argv[prAt + 1] es undefined o empieza por --, lanzar error de uso inmediatamente.
O-2 — build-prompt.js: collect.info sin línea files_captured: desactiva la comprobación de integridad en silencio
Fichero: build-prompt.js, línea ~95: const captured = Number((info.match(…) || [])[1]); seguido de if (Number.isInteger(captured) && …).
Si el regex no encuentra la línea, captured es NaN, Number.isInteger(NaN) es false, y la validación se omite. El paquete pasa sin cruzar el número de ficheros. collect.js (fuera de este diff) presumiblemente siempre la escribe, pero la comprobación existe precisamente para detectar paquetes corruptos o a medio escribir; si la línea falta, eso es exactamente un paquete corrupto, y el código lo trata como «nada que comprobar».
Corrección: Si captured no es un entero válido, lanzar error en vez de continuar.
O-3 — build-prompt.js: --out sin valor produce TypeError en vez de error de uso
Fichero: build-prompt.js, línea ~37: OUT_NAME = argv[++i]; → path.join(DIR, undefined) lanza TypeError: Path must be a string.
No es un problema de seguridad, pero es la misma familia que el --cycle NaN corregido en gate.js: un argumento inválido produce un crash opaco en vez de un mensaje de uso.
Preguntas y supuestos
No puedo verificar collect.js ni apply-files.js (fuera del alcance de esta vuelta). Asumo que collect.info siempre contiene files_captured: cuando termina con éxito; si no, O-2 es más grave de lo que parece.
No puedo ejecutar los 49 tests. No verifico que los tests de mutación descritos realmente fallen al revertir cada corrección. La afirmación es del autor.
No tengo acceso a SKILL.md ni a roles-pr197.json. No puedo confirmar que el procedimiento documentado pase --roles siempre, que es lo que mitiga B-1.
Los drivers (qwen-web-chat, glm-web-chat, kimi-web-chat) están fuera del diff. No puedo verificar si alguno imprime en stdout una línea que empiece por VEREDICTO: como parte de su banner o diagnóstico, que sería un disparador directo de M-2.
Resumen
No hay bloqueantes: ninguna ruta permite que gate.js cierre un ciclo con un veredicto falso, ni que un fichero se escriba fuera del repositorio (eso está en apply-files.js, fuera de alcance). La doble exigencia veredicto + frase única como última línea es robusta contra ecos y citas. La comprobación de PR en .meta.json cierra el hueco cross-PR.
Los dos hallazgos medios son de la clase que el PR identifica como su defecto más frecuente: un mensaje que miente (M-1) y un filtro estructural que acepta un eco del prompt como revisión (M-2). Ninguno de los dos cierra un ciclo indebidamente, pero ambos degradan la fiabilidad de lo que el árbitro y el revisor reciben como entrada. B-1 es un hueco de defensa en profundidad: la herramienta que puede cerrar una revisión no exige la comprobación de identidad, y confía en que el operador no omita un flag.
El contenido generado por IA puede no ser preciso.
