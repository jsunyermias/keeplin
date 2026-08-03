model: Qwen3.8-Max-Preview
url: https://chat.qwen.ai/c/b0b7d663-131b-47cf-95a9-e104b7a9a1ea
--- reply ---
Revisión de seguridad multimodelo
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
H-1 · collect.js — El directorio de salida no se limpia; files/ acumula ficheros de ejecuciones anteriores
Fichero y línea: collect.js, línea ≈ 57 (fs.mkdirSync(path.join(OUT, 'files'), { recursive: true })) y línea ≈ 140 (written += 1).
Garantía que rompe: La integridad del paquete como descripción autoconsistente de este PR. collect.info registra files_captured: N contando solo las escrituras de la ejecución actual, y changed-files.txt se sobreescribe con la lista actual. Pero files/ es un árbol que nunca se vacía: un fichero capturado en una ejecución previa (por ejemplo, antes de un rebase que eliminó ese fichero del PR) persiste en disco sin aparecer en changed-files.txt ni en el contador.
Escenario de fallo: Se ejecuta collect.js para el PR en el commit A (toca src/old.js). El implementador hace rebase y elimina src/old.js; se re-ejecuta collect.js en el commit B. files/src/old.js sigue en disco. changed-files.txt ya no lo lista. collect.info dice files_captured: 4 pero files/ tiene 5 entradas. Cualquier herramienta que inspeccione el directorio directamente (un ls, un find, un revisor humano que navegue el paquete) ve un fichero que el paquete dice que no existe. La dirección inversa —un fichero de la ejecución actual que no estuviera en files/— sí la detecta build-prompt.js según la descripción del autor; la dirección de exceso, no.
Corrección mínima: fs.rmSync(path.join(OUT, 'files'), { recursive: true, force: true }) inmediatamente antes del mkdirSync que lo crea, o al inicio del bloque principal tras validar los argumentos. Lo mismo aplica a OUT/context/ si el conjunto de ficheros de contexto cambiara entre ejecuciones (hoy es fijo, así que el riesgo es menor).
Lo que lo demostraría: Un test que ejecute collect.js dos veces sobre el mismo OUT con conjuntos de ficheros distintos y asserte que files/ contiene exactamente los ficheros de la segunda ejecución.
H-2 · apply-patch.js — El comentario sobre --3way describe un escenario que la estructura del código impide
Fichero y línea: apply-patch.js, línea ≈ 139–141.
1
2
// --3way lets git fall back to a merge when context has drifted, which is
// common once a review cycle has already changed the branch.
Garantía que rompe: La correspondencia entre comentario y comportamiento, que es el defecto que más ha aparecido en este PR según el propio autor.
Explicación: El flujo es: (1) git apply --check --recount → si pasa, mode = ['--recount']; (2) si no, git apply --check --recount -C1; (3) si ninguno pasa, throw; (4) git apply --3way ...mode. El paso 4 solo se alcanza si el paso 1 o 2 pasó, es decir, si el patch ya aplica limpiamente al árbol de trabajo. En esa situación --3way es un no-op: la fusión a tres bandas se degrada a la aplicación limpia que --check acaba de validar. El comentario dice que --3way rescata el caso de contexto desplazado por un ciclo de revisión, pero si el contexto se desplazó lo suficiente para impedir la aplicación limpia, --check falla y el script lanza antes de llegar a --3way. El único escenario donde --3way tendría efecto es una carrera entre el --check y el apply (el árbol cambia entre ambas llamadas), que no es lo que el comentario describe.
Corrección mínima: Reescribir el comentario para decir lo que el código hace: --3way es una red de seguridad inerte en el flujo normal; se conserva por si una carrera entre check y apply hiciera que la aplicación limpia fallara en el último momento. O eliminar el flag si se prefiere que el fallo sea explícito.
H-3 · collect.js — !== null sobre execFileSync es código muerto que sugiere un modelo mental incorrecto
Fichero y línea: collect.js, línea ≈ 97–99.
javascript
1
2
3
behind = local !== prHead
  && execFileSync('git', ['merge-base', '--is-ancestor', prHead, local],
    { cwd: REPO }) !== null;
Garantía que rompe: Ninguna funcionalmente — la lógica es correcta. Pero la comparación !== null implica que execFileSync puede devolver null en éxito, y no puede: devuelve un Buffer (sin encoding) o un string (con él), y en fallo lanza. La condición es siempre true cuando el comando termina con exit 0. Un lector que intente razonar sobre el caso null está persiguiendo un camino que no existe.
Corrección mínima: Eliminar !== null y dejar la condición como la expresión booleana que es: el éxito de execFileSync ya es la señal. O envolver en un helper isAncestor(a, b) que devuelva true/false con un try/catch.
H-4 · apply-files.js — El limpiador de chrome de UI devora líneas de contenido en la ruta sin vallas
Fichero y línea: apply-files.js, línea ≈ 48.
javascript
1
while (seg.length && /^(Copy|Copiar|[a-z]{1,12})$/i.test(seg[0].trim())) seg = seg.slice(1);
Garantía que rompe: Fidelidad del contenido escrito cuando la respuesta del implementador no trae vallas (el caso de fallback que el propio código contempla: "fences may or may not survive").
Escenario de fallo: Un fichero Python cuyo primer bloque es:
1
2
3
4
5
FICHERO: init.py
import
os
sys
def main(): ...
Sin vallas, seg = ['import', 'os', 'sys', 'def main(): ...']. El bucle: 'import' coincide con [a-z]{1,12} → fuera. 'os' coincide → fuera. 'sys' coincide → fuera. 'def main(): ...' no coincide (tiene espacios y paréntesis) → se detiene. El fichero escrito pierde sus tres primeras líneas. Es un caso límite (palabras solas de ≤12 letras al inicio), pero la ruta sin vallas existe precisamente para respuestas degradadas por el renderizador, y un fichero que empieza con directivas use, import o include en líneas separadas no es inverosímil.
Corrección mínima: Limitar el stripping a las etiquetas conocidas de UI (Copy, Copiar, y una lista cerrada de lenguajes: javascript, python, typescript, rust, bash, json, yaml, diff, patch, text, html, css, sql, go, java, c, cpp) en vez de la clase abierta [a-z]{1,12}. O exigir que la línea siguiente tras el stripping sea una valla; si no lo es, reinsertar lo stripped.
Preguntas y supuestos
No puedo verificar los tests. Los 49 tests mencionados no están en el alcance de esta revisión. No puedo confirmar que las correcciones 1–9 de la ronda anterior estén realmente cubiertas ni que los tests pasen. Lo que he revisado es solo la lógica de los tres ficheros en el diff.
No puedo ejecutar O_NOFOLLOW. La semántica del flag depende del kernel y del sistema de ficheros. En Linux ext4/tmpfs el comportamiento es el descrito (ELOOP al abrir un symlink como componente final). En otros sistemas (NFS, overlayfs) hay matices. Asumo el entorno de contenedores Linux que el código menciona.
build-prompt.js y ask.js quedan fuera. La pregunta sobre si un fallo del revisor puede confundirse con una revisión limpia se responde en esos ficheros, no en estos tres. No emito juicio sobre esa garantía.
Amenaza modelo. Asumo que el adversario es el contenido de la respuesta del implementador (rutas maliciosas, parches malformados), no un atacante concurrente con acceso al sistema de ficheros del orquestador. Las ventanas TOCTOU entre validación y escritura en apply-files.js existen formalmente pero requieren ese segundo atacante.
Resumen
Los tres ficheros son sólidos en su preocupación central: la contención de apply-files.js cubre las tres capas que describe y no he encontrado una ruta de escape que no requiera un atacante concurrente con acceso al filesystem; apply-patch.js delega correctamente en git apply sin pasar contenido por el contexto; collect.js construye un paquete coherente. Los cuatro hallazgos son observaciones: H-1 es el más concreto (directorio de salida que acumula basura entre ejecuciones y un contador que no refleja el estado real del disco); H-2 y H-3 son comentarios/código que no dicen lo que el código hace; H-4 es un caso límite en la ruta de fallback del parser. Ninguno es bloqueante por sí solo, pero H-1 merece corrección antes de fusión si el paquete se reutiliza entre ciclos.
El contenido generado por IA puede no ser preciso.
