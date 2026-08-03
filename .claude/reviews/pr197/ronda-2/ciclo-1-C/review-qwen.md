model: Qwen3.8-Max-Preview
url: https://chat.qwen.ai/c/5a95fc66-cf2f-42fd-8336-4e4a8e582d50
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
1. SKILL.md, línea ~117 del diff (paso 5): --out prompt-adjudicator.txt es una ruta relativa incoherente con el resto del procedimiento
Qué garantía rompe: La documentación del procedimiento produce un fallo de "fichero no encontrado" si se sigue al pie de la letra desde la raíz del repositorio, que es lo que todos los demás comandos del ejemplo implican.
El problema: Todos los caminos del ejemplo usan el prefijo work/pr197/:
1
node scripts/ask.js kimi work/pr197/prompt-adjudicator.txt …
Pero el --out del paso anterior es:
1
--out prompt-adjudicator.txt
Si el operador está en la raíz del repositorio (como implican work/pr197/meta.md, work/pr197/review-qwen.md, etc.), --out prompt-adjudicator.txt escribe en ./prompt-adjudicator.txt, y el ask.js del paso siguiente intenta leer work/pr197/prompt-adjudicator.txt, que no existe.
Lo que no puedo verificar: Si build-prompt.js resuelve --out relativo al primer argumento (el directorio del paquete), la ruta sería correcta. Pero eso sería un comportamiento inhabitual para un flag --out, y la SKILL.md no lo documenta. Sin acceso a build-prompt.js, no puedo confirmar ni descartar.
Corrección mínima: Cambiar a --out work/pr197/prompt-adjudicator.txt, o documentar explícitamente que --out se resuelve contra el directorio del paquete.
2. contract.test.js: no existe test para "veredicto correcto + frase ausente"
Qué garantía rompe: La SKILL.md afirma (línea ~131 del diff): "Both signals are required because either alone is forgeable." Los tests cubren: ambos presentes → cierra; veredicto bloqueante + frase → rechaza; frase citada → rechaza; frase repetida → rechaza. Pero no cubren el caso más simple de "señal ausente": VEREDICTO: SIN HALLAZGOS sin ninguna ocurrencia de la frase. Es la mitad de la conjunción que la SKILL.md declara contractual.
Escenario de fallo: Si una regresión en gate.js hiciera que la ausencia de frase se tratara como "frase válida" (por ejemplo, un lastLine.includes(PHRASE) sobre una línea vacía que no falla), ningún test lo detectaría. El test de "missing verdict" cubre la otra mitad, pero esta no.
Corrección mínima: Un test análogo al primero:
javascript
1
2
3
4
5
6
test('gate refuses a correct verdict when the phrase is entirely absent', () => {
  const f = tmpFile('r.md', 'VEREDICTO: SIN HALLAZGOS\n\nTodo correcto.\n');
  const r = run('gate.js', [f]);
  assert.strictEqual(r.code, 1);
  assert.strictEqual(JSON.parse(r.stdout).cleared, false);
});
3. SKILL.md, sección "Verification" (línea ~252 del diff): la descripción de la cobertura de apply-patch.js no coincide con lo que los tests demuestran
Qué garantía rompe: La SKILL.md dice: "apply-patch.js recovers a diff whose blank context lines were rendered away and refuses prose."
Lo que los tests realmente demuestran:
Test "applies a diff whose @@ header the model miscounted": recupera un diff con contadores de línea erróneos en @@. No se eliminan líneas en blanco del cuerpo; solo se ponen números falsos en la cabecera.
Test "refuses, and writes nothing, when a context line was lost": cuando una línea de contexto en blanco es eliminada del cuerpo (el caso real de renderizado), el script rechaza, no recupera.
La frase de la SKILL.md dice "recovers… blank context lines rendered away", que es exactamente el caso que el test demuestra que se rechaza. La recuperación es para cabeceras mal contadas, que es un caso distinto.
Corrección mínima: Reformular como: "apply-patch.js recovers a diff whose @@ header was miscounted, refuses one whose context lines were deleted by the renderer, and refuses prose."
4. contract.test.js, línea ~195: el test "ask refuses an empty reply" no pasa --pr, y podría pasar por el motivo equivocado
Qué garantía rompe: El test anterior ("ask refuses a reply with no verdict line") usa --pr 197. El test de respuesta vacía no lo usa:
javascript
1
2
const r = run('ask.js', ['codex', prompt, out], { env: stubCodex('cat >/dev/null; true') });
assert.strictEqual(r.code, 1);
Si ask.js exige --pr y sale con código 1 ante su ausencia (en vez de 2 como hace gate.js con argumentos inválidos), este test pasaría por la validación de argumentos, no por la detección de respuesta vacía. La aserción !fs.existsSync(out) se cumpliría en ambos casos.
Lo que no puedo verificar: Si --pr es opcional en ask.js (el gate es quien lo exige, no el transporte), el test es correcto. Sin acceso a ask.js, no puedo confirmar.
Corrección mínima: Añadir --pr 197 a la invocación, o añadir una aserción sobre stderr que distinga "empty reply" de "missing argument".
Preguntas y supuestos
¿--out en build-prompt.js se resuelve contra el directorio del paquete o contra CWD? De esto depende si el hallazgo 1 es un bug real o una falsa alarma. No puedo verificarlo sin la implementación.
¿--pr es opcional en ask.js? De esto depende si el hallazgo 4 es un riesgo real. La SKILL.md lo muestra siempre, pero el test de respuesta vacía lo omite.
¿Existen más ficheros de test en scripts/tests/? La cabecera del fichero dice node --test scripts/tests/ (directorio), pero la SKILL.md dice node --test scripts/tests/contract.test.js (fichero). Si hay otros, la SKILL.md estaría dando un comando incompleto. No puedo verificar el contenido del directorio.
Lo que no he podido verificar
Al no adjuntarse el texto completo de los ficheros de implementación (gate.js, roles.js, ask.js, apply-files.js, apply-patch.js, collect.js, build-prompt.js), no he podido:
Trazar las aserciones de los tests hasta el código que las satisface y confirmar que no pasan por un camino alternativo.
Verificar que gate.js exige la frase cuando el veredicto es correcto (hallazgo 2).
Verificar la resolución de --out en build-prompt.js (hallazgo 1).
Verificar si ask.js requiere --pr (hallazgo 4).
Comprobar la contención de apply-files.js más allá de lo que los tests ejercitan (por ejemplo, TOCTOU entre el lstat y el open con O_NOFOLLOW).
Ejecutar los 51 tests y confirmar que pasan.
Verificar la afirmación del PR de que "revertir cada corrección hace fallar su test" (mutación).
Resumen
No hay bloqueantes en los dos ficheros revisados. La SKILL.md es honesta sobre las limitaciones conocidas (Qwen, tamaño, eco de prompt) y el procedimiento es coherente con los tests salvo por la ruta relativa del paso 5. Los tests están bien construidos: usan spawnSync para no perder stderr, aíslan estado en directorios temporales, y los comentarios explican por qué cada aserción existe. Los tres defectos de "pasa por el motivo equivocado" que el PR describe como corregidos tienen sus correcciones visibles (el helper repoWithPullRef, el comentario sobre execFileSync, la aserción sobre stderr específico en collect). Quedan cuatro observaciones: una ruta documental incoherente, un caso contractual sin test, una descripción de cobertura imprecisa, y una posible ambigüedad en un test que no puedo resolver sin la implementación.
El contenido generado por IA puede no ser preciso.
